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

// Spectrum texture: equalizer bars drawn as characters (hero) or blocks (docs).
// Purely decorative; a static frame when motion is reduced.
(function () {
  var canvases = document.querySelectorAll("canvas[data-spectrum]");
  if (!canvases.length || !window.requestAnimationFrame) return;
  var reduce = window.matchMedia && matchMedia("(prefers-reduced-motion: reduce)").matches;
  var RAMP = ".:-=+*#%@";
  var NOISE = "01$#%&*+=?/";

  function hash(a, b) {
    var h = (a * 374761393 + b * 668265263) | 0;
    h = (h ^ (h >>> 13)) * 1274126177;
    return ((h ^ (h >>> 16)) >>> 0) / 4294967295;
  }
  function rgb(v) {
    var c = document.createElement("canvas").getContext("2d");
    c.fillStyle = v;
    var s = c.fillStyle;
    if (s.charAt(0) === "#") return [parseInt(s.substr(1, 2), 16), parseInt(s.substr(3, 2), 16), parseInt(s.substr(5, 2), 16)];
    var m = s.match(/\d+/g) || [128, 128, 128];
    return [+m[0], +m[1], +m[2]];
  }

  canvases.forEach(function (cv) {
    var mode = cv.getAttribute("data-spectrum");
    var ctx = cv.getContext("2d");
    var W = 0, H = 0, dpr = 1, cols = 0, rows = 0, cw = 0, ch = 0;
    var level = [], peak = [], visible = true, raf = 0, last = 0;
    var ink, acc;

    function colours() {
      var cs = getComputedStyle(cv);
      ink = rgb(cs.getPropertyValue("--text").trim() || "#f4f4f5");
      acc = rgb(cs.getPropertyValue("--accent").trim() || "#f59e5b");
    }
    function size() {
      var r = cv.getBoundingClientRect();
      dpr = Math.min(window.devicePixelRatio || 1, 2);
      W = r.width; H = r.height;
      cv.width = Math.round(W * dpr); cv.height = Math.round(H * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      if (mode === "blocks") { cw = Math.max(56, W / 16); ch = 26; }
      else { cw = W < 700 ? 8 : 10; ch = W < 700 ? 12 : 14; }
      cols = Math.ceil(W / cw); rows = Math.ceil(H / ch);
      for (var i = 0; i < cols; i++) { if (level[i] == null) { level[i] = 0; peak[i] = 0; } }
      colours();
    }
    // Target height of column i at time t, 0..1. Tall at the edges, quiet in the middle.
    function target(i, t) {
      if (mode === "blocks") {
        var drift = 0.5 + 0.5 * Math.sin(t * 0.35 + i * 1.7);
        return Math.max(0.12, Math.min(1, 0.15 + 0.85 * hash(i, 3) * (0.55 + 0.45 * drift)));
      }
      var x = cols > 1 ? i / (cols - 1) : 0.5;
      var edge = Math.pow(Math.abs(x - 0.5) * 2, mode === "blocks" ? 1.1 : 1.8);
      var beat = 0.5 + 0.5 * Math.pow(Math.max(0, Math.sin(t * Math.PI * 2 * 1.9)), 6);
      var wave = 0.5 + 0.22 * Math.sin(t * 1.3 + i * 0.37) + 0.16 * Math.sin(t * 2.7 - i * 0.91) + 0.12 * Math.sin(t * 0.6 + i * 0.13);
      var grain = 0.75 + 0.25 * hash(i, Math.floor(t * 6));
      var v = edge * wave * grain * (0.72 + 0.28 * beat);
      return Math.max(0, Math.min(1, mode === "blocks" ? 0.15 + v : 0.04 + v));
    }
    function draw(t) {
      ctx.clearRect(0, 0, W, H);
      var i, r, k;
      for (i = 0; i < cols; i++) {
        var tg = target(i, t);
        level[i] += (tg - level[i]) * (reduce ? 1 : 0.18);
        peak[i] = Math.max(peak[i] - 0.006, level[i]);
      }
      if (mode === "blocks") {
        var maxR = Math.min(rows, 5);
        for (i = 0; i < cols; i++) {
          var n = Math.round(level[i] * maxR);
          for (r = 0; r < n; r++) {
            var fade = 1 - r / maxR;
            var tint = hash(i, r + 99) > 0.86 ? acc : ink;
            var al = 0.035 + 0.1 * fade * (0.5 + 0.5 * hash(i, r));
            var x0 = i * cw + ((r % 2) ? cw * 0.5 : 0);
            var g = ctx.createLinearGradient(x0, 0, x0 + cw, 0);
            g.addColorStop(0, "rgba(" + tint[0] + "," + tint[1] + "," + tint[2] + "," + (al * 0.45).toFixed(3) + ")");
            g.addColorStop(1, "rgba(" + tint[0] + "," + tint[1] + "," + tint[2] + "," + al.toFixed(3) + ")");
            ctx.fillStyle = g;
            ctx.fillRect(x0, r * ch, cw - 2, ch - 2);
          }
        }
        return;
      }
      ctx.font = (ch - 2) + "px " + getComputedStyle(cv).getPropertyValue("--mono");
      ctx.textBaseline = "bottom";
      var maxRows = Math.floor(rows * 0.92);
      for (i = 0; i < cols; i++) {
        var h = Math.round(level[i] * maxRows);
        var x = i * cw;
        for (r = 0; r < h; r++) {
          var fromTop = h - 1 - r;
          var y = H - r * ch;
          var hv = hash(i, r);
          var ch1;
          if (hv > 0.93) ch1 = NOISE.charAt(Math.floor(hash(r, Math.floor(t * 3) + i) * NOISE.length));
          else ch1 = RAMP.charAt(Math.min(RAMP.length - 1, Math.floor((r / Math.max(1, h)) * 3 + (fromTop < 2 ? 0 : 3 + hv * 3))));
          if (fromTop < 2) ch1 = fromTop === 0 ? "." : ":";
          var a = 0.12 + 0.5 * (r / Math.max(8, maxRows));
          var useAcc = hv > 0.965 || (fromTop === 0 && hash(i, 7) > 0.6);
          var c = useAcc ? acc : ink;
          ctx.fillStyle = "rgba(" + c[0] + "," + c[1] + "," + c[2] + "," + (useAcc ? Math.min(0.9, a + 0.35) : a).toFixed(3) + ")";
          ctx.fillText(ch1, x, y);
        }
        // Peak hold, like a hardware meter.
        var pr = Math.round(peak[i] * maxRows);
        if (pr > h + 1) {
          ctx.fillStyle = "rgba(" + acc[0] + "," + acc[1] + "," + acc[2] + ",0.55)";
          ctx.fillText("-", x, H - pr * ch);
        }
      }
    }
    function loop(now) {
      raf = 0;
      if (!visible || document.hidden) return;
      if (now - last > 40) { last = now; draw(now / 1000); }
      raf = requestAnimationFrame(loop);
    }
    function start() { if (!raf && !reduce) raf = requestAnimationFrame(loop); }

    size();
    draw(reduce ? 4.2 : performance.now() / 1000);
    if (reduce) {
      // Settle the static frame (no easing) so it matches the motion version.
      draw(4.2);
    }
    var rt;
    window.addEventListener("resize", function () {
      clearTimeout(rt);
      rt = setTimeout(function () { size(); draw(performance.now() / 1000); }, 120);
    });
    if (window.matchMedia) {
      var mq = matchMedia("(prefers-color-scheme: light)");
      if (mq.addEventListener) mq.addEventListener("change", function () { colours(); draw(performance.now() / 1000); });
    }
    if ("IntersectionObserver" in window) {
      new IntersectionObserver(function (en) {
        visible = en[0].isIntersecting;
        if (visible) start();
      }).observe(cv);
    }
    document.addEventListener("visibilitychange", function () { if (!document.hidden) start(); });
    start();
  });
})();
