// Small enhancements only; every section works without this file.
(function () {
  var doc = document;
  var reduceMotion = window.matchMedia && matchMedia("(prefers-reduced-motion: reduce)").matches;

  // Top bar border once the page scrolls.
  var top = doc.querySelector(".top");
  if (top) {
    var onScroll = function () {
      top.classList.toggle("scrolled", window.scrollY > 8);
    };
    onScroll();
    window.addEventListener("scroll", onScroll, { passive: true });
  }

  // Copy buttons.
  doc.querySelectorAll("[data-copy]").forEach(function (btn) {
    btn.addEventListener("click", function () {
      var label = btn.textContent;
      if (!navigator.clipboard) return;
      navigator.clipboard.writeText(btn.getAttribute("data-copy")).then(function () {
        btn.textContent = btn.getAttribute("data-done") || "Copied";
        setTimeout(function () {
          btn.textContent = label;
        }, 1600);
      }, function () {});
    });
  });

  // Tabs (WAI-ARIA tabs pattern, manual activation by arrow keys).
  doc.querySelectorAll("[data-tabs]").forEach(function (box) {
    var tabs = Array.prototype.slice.call(box.querySelectorAll('[role="tab"]'));
    var select = function (tab, focus) {
      tabs.forEach(function (t) {
        var on = t === tab;
        t.setAttribute("aria-selected", on ? "true" : "false");
        t.tabIndex = on ? 0 : -1;
        var panel = doc.getElementById(t.getAttribute("aria-controls"));
        if (panel) panel.hidden = !on;
      });
      if (focus) tab.focus();
    };
    tabs.forEach(function (tab, i) {
      tab.addEventListener("click", function () {
        select(tab, false);
      });
      tab.addEventListener("keydown", function (e) {
        var next = null;
        if (e.key === "ArrowRight") next = tabs[(i + 1) % tabs.length];
        else if (e.key === "ArrowLeft") next = tabs[(i - 1 + tabs.length) % tabs.length];
        else if (e.key === "Home") next = tabs[0];
        else if (e.key === "End") next = tabs[tabs.length - 1];
        if (next) {
          e.preventDefault();
          select(next, true);
        }
      });
    });
    if (location.hash === "#build") {
      var src = box.querySelector("#tab-source");
      if (src) select(src, false);
    }
  });

  // A soft light that follows the pointer over cards.
  if (window.matchMedia && matchMedia("(hover: hover) and (pointer: fine)").matches) {
    doc.addEventListener("pointermove", function (e) {
      var card = e.target.closest && e.target.closest(".card");
      if (!card) return;
      var r = card.getBoundingClientRect();
      card.style.setProperty("--mx", e.clientX - r.left + "px");
      card.style.setProperty("--my", e.clientY - r.top + "px");
    }, { passive: true });
  }

  // Fade sections in as they scroll into view.
  var items = doc.querySelectorAll(".reveal");
  if (!reduceMotion && "IntersectionObserver" in window) {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) {
        if (en.isIntersecting) {
          en.target.classList.add("in");
          io.unobserve(en.target);
        }
      });
    }, { rootMargin: "0px 0px -8% 0px", threshold: 0.08 });
    items.forEach(function (el) {
      io.observe(el);
    });
    // Never leave content hidden if the observer does not run.
    setTimeout(function () {
      items.forEach(function (el) {
        el.classList.add("in");
      });
    }, 2500);
  } else {
    items.forEach(function (el) {
      el.classList.add("in");
    });
  }

  // On Windows on ARM, put the ARM64 installer first.
  var uad = navigator.userAgentData;
  if (uad && uad.platform === "Windows" && uad.getHighEntropyValues) {
    uad.getHighEntropyValues(["architecture"]).then(function (v) {
      if (v.architecture !== "arm") return;
      doc.querySelectorAll("[data-arch-swap]").forEach(function (group) {
        var x64 = group.querySelector('[data-arch="x64"]');
        var arm = group.querySelector('[data-arch="arm64"]');
        if (!x64 || !arm) return;
        ["btn-primary", "primary"].forEach(function (c) {
          if (x64.classList.contains(c)) {
            x64.classList.remove(c);
            arm.classList.add(c);
          }
        });
        if (arm.classList.contains("btn-primary")) {
          arm.classList.remove("btn-quiet");
          x64.classList.add("btn-quiet");
          x64.classList.remove("btn-primary");
        }
        group.insertBefore(arm, x64);
      });
    }, function () {});
  }

  // Live repository data from the public GitHub API, cached for an hour.
  // The page already shows the last known version; this only refreshes it.
  var KEY = "openify-gh-v1";
  var apply = function (d) {
    if (!d) return;
    if (d.version) {
      doc.querySelectorAll('[data-gh="version"]').forEach(function (el) {
        el.textContent = d.version;
      });
    }
    if (typeof d.stars === "number" && d.stars >= 5) {
      var n = d.stars >= 1000 ? (d.stars / 1000).toFixed(1).replace(/\.0$/, "") + "k" : String(d.stars);
      doc.querySelectorAll('[data-gh="stars"]').forEach(function (el) {
        el.textContent = n;
      });
      doc.querySelectorAll('[data-gh="stars-wrap"], [data-gh="stars-li"]').forEach(function (el) {
        el.hidden = false;
      });
    }
  };
  var cached = null;
  try {
    cached = JSON.parse(sessionStorage.getItem(KEY) || "null");
  } catch (e) {}
  if (cached && Date.now() - cached.t < 3600000) {
    apply(cached);
  } else if (window.fetch) {
    var base = "https://api.github.com/repos/FurkanKY/Openify";
    var get = function (url) {
      return fetch(url, { headers: { Accept: "application/vnd.github+json" } }).then(function (r) {
        return r.ok ? r.json() : null;
      });
    };
    Promise.all([get(base).catch(function () { return null; }), get(base + "/releases/latest").catch(function () { return null; })]).then(function (res) {
      var d = { t: Date.now() };
      if (res[0] && typeof res[0].stargazers_count === "number") d.stars = res[0].stargazers_count;
      if (res[1] && res[1].tag_name) d.version = res[1].tag_name;
      apply(d);
      try {
        sessionStorage.setItem(KEY, JSON.stringify(d));
      } catch (e) {}
    });
  }
})();
