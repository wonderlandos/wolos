// SPDX-License-Identifier: BSD-2-Clause
// The page a system web app shows when it cannot show itself (ADR 0048, QA-WEB-9):
//   error.html?reason=crashed|manifest|kit|refused|channel&name=<app name>&retry=<file: address>
// The host (crates/webengine/src/bin/wol-webhost.rs) loads it in place of the app. It needs
// nothing else of the kit (the reason may be that the rest is missing), only words and one button:
// Try again goes to the address the host gave, which is the app's own page, where the runtime
// channel is set up again. Every string is here; a reason this table does not know is the generic one.
(function () {
  "use strict";
  var WORDS = {
    crashed: ["{name} stopped", "The page stopped working. Nothing you did is lost."],
    manifest: ["{name} cannot start", "WolOS could not read the description of this app."],
    kit: ["{name} cannot start", "A part of WolOS that this app needs is missing."],
    refused: ["{name} stayed where it was", "WolOS did not let the app open an address outside itself."],
    channel: ["{name} is not connected to WolOS", "This app can only run inside WolOS."],
    generic: ["Something went wrong", "The app could not carry on."]
  };
  var AGAIN = "Try again";
  var q = new URLSearchParams(location.search);
  var reason = q.get("reason") || "generic";
  var words = WORDS[Object.prototype.hasOwnProperty.call(WORDS, reason) ? reason : "generic"];
  var name = (q.get("name") || "This app").slice(0, 80);
  var retry = q.get("retry") || "";
  // Try again goes only to a page of an app beside this kit: file:///<apps folder>/<slug>/<page>.html,
  // the address read as the engine reads it (so %2e%2e and ../ are resolved before the check), no
  // query, no fragment, never the kit's own folder.
  function own(address) {
    try {
      var root = new URL("../", location.href).pathname;
      var u = new URL(address);
      if (u.protocol !== "file:" || u.search || u.hash || u.pathname.indexOf(root) !== 0) return false;
      var rest = u.pathname.slice(root.length);
      return /^[a-z0-9-]+\/[^\/%\\]+\.html$/.test(rest) && rest.split("/")[0] !== "kit";
    } catch (x) { return false; }
  }
  var ok = own(retry);
  if (ok) retry = new URL(retry).href;
  function fill() {
    document.title = words[0].replace("{name}", name);
    document.getElementById("title").textContent = words[0].replace("{name}", name);
    document.getElementById("lead").textContent = words[1];
    var b = document.getElementById("again");
    b.textContent = AGAIN;
    if (!ok) { document.getElementById("again-row").hidden = true; return; }
    b.addEventListener("click", function () { location.href = retry; });
    b.focus();
  }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", fill); else fill();
  window.__wolosError = Object.freeze({ reason: reason, title: words[0], lead: words[1], retry: ok ? retry : "" });
})();
