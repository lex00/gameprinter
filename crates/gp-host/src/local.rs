//! The host that runs in this process: sessions in memory, a worker pool for
//! background jobs, links in a JSON file under the host's directory.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use gp_mcp::{Config, CopyLock};
use serde_json::{Value, json};

use crate::links::{self, Link};
use crate::scenario;
use crate::session::Session;
use crate::speed;
use crate::{Host, JobSpec, JobState, JobView, StepRequest};

/// The most sessions open at once.
pub const MAX_SESSIONS: usize = 32;

/// Where the host keeps things and how much it runs at once.
#[derive(Debug, Clone)]
pub struct HostConfig {
    /// The tools' configuration: `work` holds the working copies.
    pub mcp: Config,
    /// The host's own directory: links, build outputs, snapshots, the shared
    /// Bevy target directory. Outside any git work tree.
    pub dir: PathBuf,
    /// Background builds at once. Builds share one cargo target directory,
    /// so more than one mostly waits on cargo's lock.
    pub build_workers: usize,
}

impl HostConfig {
    pub fn new(mcp: Config) -> Self {
        let dir = mcp.work.join(".host");
        Self {
            mcp,
            dir,
            build_workers: 1,
        }
    }
}

struct JobRec {
    id: String,
    spec: JobSpec,
    state: JobState,
    queued: Instant,
    started: Option<Instant>,
    finished: Option<Instant>,
    result: Option<Value>,
    error: Option<String>,
    link: Option<Link>,
}

#[derive(Default)]
struct Queue {
    jobs: BTreeMap<String, JobRec>,
    pending: VecDeque<String>,
}

pub struct LocalHost {
    cfg: HostConfig,
    sessions: Mutex<BTreeMap<String, Arc<Mutex<Session>>>>,
    queue: Mutex<Queue>,
    wake: Condvar,
    links: Mutex<BTreeMap<String, Link>>,
    base: Mutex<Option<String>>,
    next: AtomicU64,
    speed: Mutex<Vec<(String, f64)>>,
    /// The host itself, for jobs that run on threads of their own.
    me: Mutex<std::sync::Weak<LocalHost>>,
}

impl LocalHost {
    /// Starts a host and its workers. The workers run until the process
    /// ends.
    pub fn start(cfg: HostConfig) -> Result<Arc<Self>, String> {
        std::fs::create_dir_all(&cfg.dir)
            .map_err(|e| format!("creating {}: {e}", cfg.dir.display()))?;
        let links = links::load(&cfg.dir.join("links.json"))
            .into_iter()
            .filter(|(_, l)| l.dir.join("manifest.json").is_file())
            .collect();
        let workers = cfg.build_workers.max(1);
        let host = Arc::new(Self {
            cfg,
            sessions: Mutex::default(),
            queue: Mutex::default(),
            wake: Condvar::new(),
            links: Mutex::new(links),
            base: Mutex::new(None),
            next: AtomicU64::new(1),
            speed: Mutex::default(),
            me: Mutex::new(std::sync::Weak::new()),
        });
        *lock(&host.me) = Arc::downgrade(&host);
        for _ in 0..workers {
            let h = Arc::clone(&host);
            std::thread::spawn(move || h.work());
        }
        Ok(host)
    }

    pub fn config(&self) -> &HostConfig {
        &self.cfg
    }

    /// Tells the host its URL, so links carry it.
    pub fn set_base(&self, base: &str) {
        *lock(&self.base) = Some(base.trim_end_matches('/').to_owned());
    }

    pub fn link(&self, id: &str) -> Option<Link> {
        lock(&self.links).get(id).cloned()
    }

    fn with_url(&self, mut l: Link) -> Link {
        l.url = lock(&self.base).as_ref().map(|b| format!("{b}{}", l.path));
        l
    }

    fn id(&self, prefix: &str) -> String {
        format!("{prefix}{}", self.next.fetch_add(1, Ordering::Relaxed))
    }

    fn note(&self, name: &str, seconds: f64) {
        lock(&self.speed).push((name.to_owned(), seconds));
    }

    /// A worker: takes the next queued job and runs it.
    fn work(self: Arc<Self>) {
        loop {
            let (id, spec) = {
                let mut q = lock(&self.queue);
                loop {
                    if let Some(id) = q.pending.pop_front() {
                        let rec = q.jobs.get_mut(&id).expect("a queued job has a record");
                        rec.state = JobState::Running;
                        rec.started = Some(Instant::now());
                        break (id, rec.spec.clone());
                    }
                    q = self.wake.wait(q).unwrap_or_else(|e| e.into_inner());
                }
            };
            let out = self.execute(&id, &spec);
            self.finish(&id, out);
        }
    }

    /// Records how a job ended.
    fn finish(&self, id: &str, out: Result<(Value, Option<Link>), String>) {
        {
            let mut q = lock(&self.queue);
            let rec = q.jobs.get_mut(id).expect("a running job has a record");
            rec.finished = Some(Instant::now());
            match out {
                Ok((result, link)) => {
                    rec.state = JobState::Done;
                    rec.result = Some(result);
                    rec.link = link;
                }
                Err(e) => {
                    rec.state = JobState::Failed;
                    rec.error = Some(e);
                }
            }
            if let (JobState::Done, Some(started)) = (rec.state, rec.started) {
                let name = match rec.spec.kind() {
                    "build" => "build_s",
                    "scenarios" => "scenarios_s",
                    _ => return,
                };
                let secs = (rec.finished.unwrap_or(started) - started).as_secs_f64();
                drop(q);
                self.note(name, secs);
            }
        }
    }

    /// Runs a capture job: `gp capture start` when asked, then `gp capture
    /// watch` until the run is steady, as child processes in the repo with
    /// the host's `$GP_HOME`. The run's events are the job's progress; its
    /// result is the watcher's answer.
    fn capture(
        &self,
        run: &str,
        start: Option<&crate::CaptureStart>,
        watch: &[String],
    ) -> Result<(Value, Option<Link>), String> {
        let gp = |args: &[String]| -> Result<(i32, Value, String), String> {
            let out = std::process::Command::new(&self.cfg.mcp.gp)
                .arg("--json")
                .args(args)
                .current_dir(&self.cfg.mcp.repo)
                .env("GP_HOME", &self.cfg.mcp.home)
                .stdin(std::process::Stdio::null())
                .output()
                .map_err(|e| format!("running gp: {e}"))?;
            Ok((
                out.status.code().unwrap_or(2),
                serde_json::from_slice(&out.stdout).unwrap_or(Value::Null),
                String::from_utf8_lossy(&out.stderr).into_owned(),
            ))
        };
        let mut started = Value::Null;
        if let Some(s) = start {
            let (code, j, err) = gp(&s.args(run))?;
            if code == 2 {
                return Err(format!(
                    "gp capture start: {}",
                    err.lines().last().unwrap_or("")
                ));
            }
            started = j;
        }
        let mut args = vec!["capture".to_owned(), "watch".into(), run.into()];
        args.extend(watch.iter().cloned());
        let (code, j, err) = gp(&args)?;
        if code == 2 {
            return Err(format!(
                "gp capture watch: {}",
                err.lines().last().unwrap_or("")
            ));
        }
        Ok((json!({ "run": run, "start": started, "watch": j }), None))
    }

    fn execute(&self, id: &str, spec: &JobSpec) -> Result<(Value, Option<Link>), String> {
        match spec {
            JobSpec::Capture {
                run,
                start,
                debounce_ms,
                max_rounds,
                idle_s,
            } => {
                let mut watch = Vec::new();
                for (flag, v) in [
                    ("--debounce-ms", debounce_ms.map(|v| v.to_string())),
                    ("--max-rounds", max_rounds.map(|v| v.to_string())),
                    ("--idle-s", idle_s.map(|v| v.to_string())),
                ] {
                    if let Some(v) = v {
                        watch.extend([flag.to_owned(), v]);
                    }
                }
                self.capture(run, start.as_ref(), &watch)
            }
            JobSpec::Scenarios {
                copy,
                template,
                names,
                jobs,
            } => {
                let r = self.run_scenarios(&scenario::Request {
                    copy: copy.clone(),
                    template: *template,
                    names: names.clone(),
                    jobs: *jobs,
                })?;
                Ok((r, None))
            }
            JobSpec::Build {
                copy,
                target,
                fetch_tools,
            } => {
                let target = target.clone().unwrap_or_else(|| "web-preview".into());
                let cfg = self.snapshot(copy, id)?;
                let out = self.cfg.dir.join("builds").join(copy).join(id);
                let reply = gp_mcp::publish::publish(
                    &cfg,
                    gp_mcp::publish::PublishArgs {
                        copy: copy.clone(),
                        target: Some(target),
                        out: Some(out.clone()),
                        dry_run: false,
                        fetch_tools: *fetch_tools,
                    },
                );
                // The snapshot is ours and has been built from.
                let _ = std::fs::remove_dir_all(&cfg.work);
                let reply = reply?;
                if reply.is_error || reply.json["written"] != true {
                    return Err(format!("the build was refused: {}", reply.json));
                }
                let link = self.register_link(&out)?;
                Ok((reply.json, Some(link)))
            }
        }
    }

    /// A copy of the working copy under the host's directory, with a
    /// configuration that points at it, so a build is of the copy as it was
    /// when the job started and edits don't wait for it. The cargo target
    /// directory for the Bevy build is shared by symlink, since a cold
    /// build takes minutes.
    fn snapshot(&self, copy: &str, job: &str) -> Result<Config, String> {
        let src = {
            let _lock = CopyLock::take(copy);
            let src = self.cfg.mcp.existing_copy(copy)?;
            let work = self.cfg.dir.join("snapshots").join(job);
            copy_tree(&src, &work.join(copy))?;
            work
        };
        let shared = self.cfg.dir.join("bevy-target");
        std::fs::create_dir_all(&shared).map_err(|e| e.to_string())?;
        let publish = src.join(".publish");
        std::fs::create_dir_all(&publish).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&shared, publish.join("bevy-target"))
            .map_err(|e| e.to_string())?;
        let mut cfg = self.cfg.mcp.clone();
        cfg.work = src;
        Ok(cfg)
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("creating {}: {e}", to.display()))?;
    for e in std::fs::read_dir(from).map_err(|e| format!("reading {}: {e}", from.display()))? {
        let e = e.map_err(|e| e.to_string())?;
        let (src, dst) = (e.path(), to.join(e.file_name()));
        let t = e.file_type().map_err(|e| e.to_string())?;
        if t.is_dir() {
            copy_tree(&src, &dst)?;
        } else if t.is_file() {
            std::fs::copy(&src, &dst).map_err(|e| format!("copying {}: {e}", src.display()))?;
        }
    }
    Ok(())
}

fn view(r: &JobRec) -> JobView {
    let now = Instant::now();
    JobView {
        id: r.id.clone(),
        kind: r.spec.kind(),
        copy: r.spec.copy().to_owned(),
        state: r.state,
        wait_ms: r
            .started
            .unwrap_or(now)
            .duration_since(r.queued)
            .as_millis(),
        run_ms: r.started.map_or(0, |s| {
            r.finished.unwrap_or(now).duration_since(s).as_millis()
        }),
        result: r.result.clone(),
        error: r.error.clone(),
        link: r.link.clone(),
    }
}

impl Host for LocalHost {
    fn open_session(&self, copy: &str) -> Result<Value, String> {
        let mut s = lock(&self.sessions);
        if s.len() >= MAX_SESSIONS {
            return Err(format!("{MAX_SESSIONS} sessions are open; close one"));
        }
        let id = self.id("s");
        let session = Session::open(&self.cfg.mcp, id.clone(), copy)?;
        let info = session.info();
        s.insert(id, Arc::new(Mutex::new(session)));
        Ok(info)
    }

    fn step_session(&self, id: &str, step: &StepRequest) -> Result<Value, String> {
        let s = lock(&self.sessions)
            .get(id)
            .cloned()
            .ok_or(format!("there's no session `{id}`"))?;
        let mut s = lock(&s);
        s.step(&self.cfg.mcp, step)
    }

    fn query_session(&self, id: &str, paths: &[String]) -> Result<Value, String> {
        let s = lock(&self.sessions)
            .get(id)
            .cloned()
            .ok_or(format!("there's no session `{id}`"))?;
        let mut s = lock(&s);
        s.query(&self.cfg.mcp, paths)
    }

    fn close_session(&self, id: &str) -> Result<(), String> {
        lock(&self.sessions)
            .remove(id)
            .map(|_| ())
            .ok_or(format!("there's no session `{id}`"))
    }

    fn sessions(&self) -> Vec<Value> {
        let all: Vec<_> = lock(&self.sessions).values().cloned().collect();
        all.iter().map(|s| lock(s).info()).collect()
    }

    fn run_scenarios(&self, req: &scenario::Request) -> Result<Value, String> {
        let dir = if req.template {
            if !gp_model::path::is_name(&req.copy.replace('-', "_")) {
                return Err(format!("`{}` isn't a template id", req.copy));
            }
            self.cfg.mcp.repo.join("templates").join(&req.copy)
        } else {
            self.cfg.mcp.existing_copy(&req.copy)?
        };
        scenario::run_all(&self.cfg.mcp, &dir, req)
    }

    fn record_scenario(&self, req: &scenario::Record) -> Result<Value, String> {
        let _lock = CopyLock::take(&req.copy);
        let dir = self.cfg.mcp.existing_copy(&req.copy)?;
        scenario::record(&self.cfg.mcp, &dir, req)
    }

    fn search_inputs(&self, req: &crate::search::Search) -> Result<Value, String> {
        let dir = self.cfg.mcp.existing_copy(&req.copy)?;
        crate::search::search(&self.cfg.mcp, &dir, req)
    }

    fn submit(&self, spec: JobSpec) -> Result<String, String> {
        // Refuse what can never run, so the job doesn't just fail later.
        if let JobSpec::Build { copy, target, .. } = &spec {
            self.cfg.mcp.existing_copy(copy)?;
            if let Some(t) = target
                && !matches!(t.as_str(), "web-preview" | "web")
            {
                return Err(format!("target `{t}` isn't `web-preview` or `web`"));
            }
        }
        if let JobSpec::Capture { run, start, .. } = &spec {
            let ok = !run.is_empty()
                && run.len() <= 80
                && run
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !ok {
                return Err(format!("`{run}` isn't a capture run name"));
            }
            let exists = self
                .cfg
                .mcp
                .home
                .join("capture")
                .join(run)
                .join("run.json")
                .is_file();
            if start.is_none() && !exists {
                return Err(format!(
                    "there's no capture run `{run}`; give `start` to begin one"
                ));
            }
            if start.is_some() && exists {
                return Err(format!("the capture run `{run}` exists; leave out `start`"));
            }
        }
        let id = self.id("j");
        let own_thread = matches!(spec, JobSpec::Capture { .. });
        let mut q = lock(&self.queue);
        q.jobs.insert(
            id.clone(),
            JobRec {
                id: id.clone(),
                spec,
                state: JobState::Queued,
                queued: Instant::now(),
                started: None,
                finished: None,
                result: None,
                error: None,
                link: None,
            },
        );
        if own_thread {
            // A capture job waits on a person for minutes; it doesn't hold
            // a build worker while it does.
            let rec = q.jobs.get_mut(&id).expect("just inserted");
            rec.state = JobState::Running;
            rec.started = Some(Instant::now());
            let spec = rec.spec.clone();
            drop(q);
            let host = self.me.lock().ok().and_then(|m| m.upgrade());
            let Some(host) = host else {
                return Err("the host isn't running".into());
            };
            let job = id.clone();
            std::thread::spawn(move || {
                let out = host.execute(&job, &spec);
                host.finish(&job, out);
            });
            return Ok(id);
        }
        q.pending.push_back(id.clone());
        self.wake.notify_one();
        Ok(id)
    }

    fn job(&self, id: &str) -> Option<JobView> {
        lock(&self.queue).jobs.get(id).map(view).map(|mut v| {
            v.link = v.link.map(|l| self.with_url(l));
            v
        })
    }

    fn jobs(&self) -> Vec<JobView> {
        let mut v: Vec<JobView> = lock(&self.queue).jobs.values().map(view).collect();
        for j in &mut v {
            j.link = j.link.take().map(|l| self.with_url(l));
            // A listing is a status board, not the results.
            j.result = None;
        }
        v
    }

    fn register_link(&self, dir: &Path) -> Result<Link, String> {
        let link = links::link_for(dir)?;
        let mut all = lock(&self.links);
        all.insert(link.id.clone(), link.clone());
        links::save(&self.cfg.dir.join("links.json"), &all)?;
        drop(all);
        Ok(self.with_url(link))
    }

    fn links(&self) -> Vec<Link> {
        let all: Vec<Link> = lock(&self.links).values().cloned().collect();
        all.into_iter().map(|l| self.with_url(l)).collect()
    }

    fn speed(&self) -> Value {
        let samples = lock(&self.speed).clone();
        let mut by: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
        for (n, s) in &samples {
            by.entry(n.as_str()).or_default().push(*s);
        }
        let mut out = serde_json::Map::new();
        for (n, v) in by {
            let last = *v.last().expect("a sample");
            let mut e = json!({
                "samples": v.len(),
                "last_s": last,
                "max_s": v.iter().cloned().fold(0.0, f64::max),
            });
            if let Ok(verdict) = speed::judge(n, last) {
                e["budget_s"] = json!(verdict.budget_s);
                e["target_s"] = json!(verdict.target_s);
                e["within_budget"] = json!(verdict.within_budget);
            }
            out.insert(n.to_owned(), e);
        }
        json!({ "metrics": out, "targets": speed::TARGETS })
    }

    fn record_speed(&self, name: &str, seconds: f64) -> Result<Value, String> {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err("seconds must be a number of zero or more".into());
        }
        let verdict = speed::judge(name, seconds)?;
        self.note(name, seconds);
        Ok(serde_json::to_value(verdict).expect("a verdict serializes"))
    }
}
