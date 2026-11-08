// The public page's first script (docs/web.md, "The public page"). It is
// a classic script, not a module, and written in old JavaScript, so it
// runs in any browser that runs scripts, even one where the page's module
// (page.js) fails to load, parse or link. Such a module fails with no
// word: no handler is attached, and "Load your own ROM" opens the picker
// and then does nothing (the user, 2026-11-07, in Safari). This file:
//
// - takes the file picked with "Load your own ROM" itself, on `change` or
//   `input` (whichever comes first), says "Checking NAME..." at once and
//   hands the file to page.js (`__gpBoot.take`), or keeps it until page.js
//   is ready;
// - writes any uncaught error or rejected promise of the page to #message,
//   in words;
// - says so on the page when page.js fails to load, or hasn't marked
//   itself ready (`__gpBoot.markReady`) a few seconds after this file ran.
//
// The Content-Security-Policy allows scripts from the page's own files
// only, so this is a file, not an inline script.
(function () {
  "use strict";
  var WAIT_MS = 6000;
  var boot = (window.__gpBoot = {
    ready: false,
    pending: null,
    take: null,
    failed: "",
  });
  var bootMessage = "";

  function say(text, kind) {
    var m = document.getElementById("message");
    if (!m) return;
    m.textContent = text;
    m.setAttribute("data-kind", kind || "");
  }
  // An error in words, without its own closing period (the sentence it
  // goes into has one).
  function describe(x) {
    var text = x && typeof x === "object" && x.message ? String(x.message) : String(x);
    return text.replace(/\.+\s*$/, "");
  }
  // A message of this file's, which page.js clears when it becomes ready.
  function sayBoot(text) {
    bootMessage = text;
    say(text, "error");
  }

  function notStarted(why) {
    boot.failed = why;
    sayBoot(
      "The page's script " +
        why +
        ", so the game and \"Load your own ROM\" can't run in this browser. Reload the page; if this message stays, " +
        "try a newer version of this browser or another browser."
    );
    if (boot.pending) cantRead(boot.pending.name);
  }
  function cantRead(name) {
    sayBoot(
      name +
        " can't be read: the page's script " +
        boot.failed +
        ". Reload the page and load the file again; if this message stays, try a newer version of this browser or another browser."
    );
  }

  // page.js calls this as it starts. It clears a message of this file's
  // (a slow start that ended well) and takes a file picked before it was
  // ready.
  boot.markReady = function () {
    boot.ready = true;
    boot.failed = "";
    var m = document.getElementById("message");
    if (bootMessage && m && m.textContent === bootMessage) say("", "");
    bootMessage = "";
    var p = boot.pending;
    boot.pending = null;
    if (p && boot.take) boot.take(p.file, p.name);
  };

  // The ROM picker. Both events are listened to: a browser that sends only
  // one of them still loads the file. Taking it the first time clears the
  // input, so the second event finds no file, and picking the same file
  // again sends both again.
  function picked(e) {
    var input = e.target;
    var file = input.files && input.files[0];
    if (!file) return;
    try {
      input.value = "";
    } catch (err) {
      // An old browser that can't clear it: the same file picked again may
      // send no event.
    }
    say("Checking " + file.name + "...", "");
    if (boot.ready && boot.take) {
      boot.take(file, file.name);
      return;
    }
    boot.pending = { file: file, name: file.name };
    if (boot.failed) cantRead(file.name);
  }
  var rom = document.getElementById("rom");
  if (rom) {
    rom.addEventListener("change", picked);
    rom.addEventListener("input", picked);
  }

  // Uncaught errors and rejections anywhere in the page (not the game's
  // frame, which shows its own): in words, never only in the console.
  // A script or stylesheet of the page that fails to load sends an error
  // event that doesn't bubble, so it is taken while it goes down.
  window.addEventListener(
    "error",
    function (e) {
      var t = e.target;
      if (t && t !== window && t.tagName) {
        var src = t.getAttribute("src") || t.getAttribute("href") || t.tagName.toLowerCase();
        if (t.tagName === "SCRIPT" && /page\.js$/.test(src)) notStarted("(page.js) didn't load");
        else if (t.tagName === "SCRIPT" || t.tagName === "LINK") say("A file of the page didn't load: " + src + ". Reload the page to try again.", "error");
        return;
      }
      var text = describe(e.error || e.message || "an unknown error");
      // Chrome reports a resize it put off to the next frame as an error;
      // nothing went wrong.
      if (/ResizeObserver loop/.test(text)) return;
      // Before page.js is ready the only scripts here are this file and
      // page.js with its imports, so an error then is page.js failing to
      // link or start (Safari names no file for a missing import).
      if (!boot.ready) {
        notStarted("(page.js) stopped as it started: " + text);
        return;
      }
      say("Something went wrong on the page: " + text + ". Reload the page to try again.", "error");
    },
    true
  );
  window.addEventListener("unhandledrejection", function (e) {
    say("Something went wrong on the page: " + describe(e.reason) + ". Reload the page to try again.", "error");
  });

  setTimeout(function () {
    if (!boot.ready && !boot.failed) notStarted("didn't start");
  }, WAIT_MS);
})();
