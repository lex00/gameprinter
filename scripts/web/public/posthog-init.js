// Counts the public page's visits and records its sessions with PostHog
// (the About box says so). posthog.js is posthog-js's
// array.full.no-external.js, pinned in scripts/web/package.json and copied
// beside this file by build-page.sh: the whole library in one file that
// loads nothing from elsewhere, since the Content-Security-Policy allows
// scripts from the page's own files only, and none inline. Events go to
// PostHog's https address, which connect-src allows.
//
// A page served from this machine (the release check's, `gp registry
// serve`'s) sends nothing.
(function () {
  "use strict";
  var host = location.hostname;
  if (!window.posthog || host === "localhost" || host === "127.0.0.1" || host === "[::1]") return;
  window.posthog.init("phc_nYRaxsTgkbkKrWkQMH2wnKDhfwPS4f2WmB7zS5exsTWo", {
    api_host: "https://us.i.posthog.com",
    defaults: "2026-05-30",
    disable_external_dependency_loading: true
  });
})();
