// Small enhancements only; the page works without this file.
(function () {
  var top = document.querySelector(".top");
  if (top) {
    var onScroll = function () {
      top.classList.toggle("scrolled", window.scrollY > 8);
    };
    onScroll();
    window.addEventListener("scroll", onScroll, { passive: true });
  }

  document.querySelectorAll("[data-copy]").forEach(function (btn) {
    btn.addEventListener("click", function () {
      var text = btn.getAttribute("data-copy");
      var label = btn.textContent;
      var done = function () {
        btn.textContent = btn.getAttribute("data-done") || "Copied";
        setTimeout(function () {
          btn.textContent = label;
        }, 1600);
      };
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(text).then(done, function () {});
      }
    });
  });

  // On Windows on ARM, put the ARM64 installer first.
  var uad = navigator.userAgentData;
  if (uad && uad.platform === "Windows" && uad.getHighEntropyValues) {
    uad.getHighEntropyValues(["architecture"]).then(function (v) {
      if (v.architecture !== "arm") return;
      document.querySelectorAll("[data-arch-swap]").forEach(function (group) {
        var x64 = group.querySelector("[data-arch=x64]");
        var arm = group.querySelector("[data-arch=arm64]");
        if (!x64 || !arm) return;
        x64.classList.replace("btn-primary", "btn-quiet");
        arm.classList.replace("btn-quiet", "btn-primary");
        group.insertBefore(arm, x64);
      });
    }, function () {});
  }
})();
