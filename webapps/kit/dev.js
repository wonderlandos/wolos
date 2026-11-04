// SPDX-License-Identifier: BSD-2-Clause
// Development mode loader (ADR 0048): with ?dev in the address, load the scripts named in
// data-dev (mock bridge and fixtures, which the package leaves out); otherwise load nothing, so
// a shipped page makes no request for a file that is not there (QA-WEB-11).
//   <script src="../kit/dev.js" data-dev="../kit/mock-bridge.js ../kit/fixtures.js fixtures.js"></script>
(function () {
  "use strict";
  var me = document.currentScript;
  if (!me || !/[?&]dev(=|&|$)/.test(location.search)) return;
  (me.getAttribute("data-dev") || "").split(/\s+/).forEach(function (src) {
    if (src && !/["<>]/.test(src)) document.write('<script src="' + src + '"><\/script>');
  });
})();
