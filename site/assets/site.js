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
      // Swap what the two links point to: the big button becomes the ARM64
      // installer and the small link offers x64 instead.
      doc.querySelectorAll("[data-arch-swap]").forEach(function (group) {
        var x64 = group.querySelector('[data-arch="x64"]');
        var arm = group.querySelector('[data-arch="arm64"]');
        if (!x64 || !arm) return;
        var h = x64.getAttribute("href");
        x64.setAttribute("href", arm.getAttribute("href"));
        arm.setAttribute("href", h);
        var small = x64.querySelector("small");
        if (small && x64.dataset.smallAlt) small.textContent = x64.dataset.smallAlt;
        if (x64.dataset.linkAlt) arm.textContent = x64.dataset.linkAlt;
        x64.dataset.arch = "arm64";
        arm.dataset.arch = "x64";
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
      buildAtlas();
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
    var ease = 0.18, decay = 0.006;
    function draw(t, dt) {
      if (dt) { ease = 1 - Math.exp(-dt * 5); decay = dt * 0.15; }
      ctx.clearRect(0, 0, W, H);
      var i, r, k;
      for (i = 0; i < cols; i++) {
        var tg = target(i, t);
        level[i] += (tg - level[i]) * (reduce ? 1 : ease);
        peak[i] = Math.max(peak[i] - decay, level[i]);
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
      // Glyphs come from a pre-rendered atlas: one drawImage per cell instead of fillText.
      var maxRows = Math.floor(rows * 0.66);
      for (i = 0; i < cols; i++) {
        var h = Math.min(maxRows, Math.round(level[i] * maxRows));
        var x = i * cw;
        for (r = 0; r < h; r++) {
          var fromTop = h - 1 - r;
          var y = H - (r + 1) * ch;
          var hv = hash(i, r);
          var gi;
          if (fromTop < 2) gi = fromTop === 0 ? 0 : 1;
          else if (hv > 0.93) gi = RAMP.length + Math.floor(hash(r, Math.floor(t * 3) + i) * NOISE.length);
          else gi = Math.min(RAMP.length - 1, Math.floor((r / Math.max(1, h)) * 3 + 3 + hv * 3));
          var a = 0.12 + 0.5 * (r / Math.max(8, maxRows));
          var useAcc = hv > 0.965 || (fromTop === 0 && hash(i, 7) > 0.6);
          ctx.globalAlpha = useAcc ? Math.min(0.9, a + 0.35) : a;
          ctx.drawImage(atlas, gi * aw, useAcc ? ah : 0, aw, ah, x, y, cw, ch);
        }
        var pr = Math.min(maxRows, Math.round(peak[i] * maxRows));
        if (pr > h + 1) {
          ctx.globalAlpha = 0.55;
          ctx.drawImage(atlas, GLYPHS.length * aw - aw, ah, aw, ah, x, H - pr * ch, cw, ch);
        }
      }
      ctx.globalAlpha = 1;
    }
    var GLYPHS = RAMP + NOISE + "-", atlas = null, aw = 0, ah = 0;
    function buildAtlas() {
      if (mode === "blocks") return;
      aw = Math.ceil(cw * dpr); ah = Math.ceil(ch * dpr);
      atlas = document.createElement("canvas");
      atlas.width = aw * GLYPHS.length; atlas.height = ah * 2;
      var a = atlas.getContext("2d");
      a.font = Math.round((ch - 2) * dpr) + "px " + (getComputedStyle(cv).getPropertyValue("--mono") || "monospace");
      a.textBaseline = "bottom";
      [ink, acc].forEach(function (c, row) {
        a.fillStyle = "rgb(" + c[0] + "," + c[1] + "," + c[2] + ")";
        for (var k = 0; k < GLYPHS.length; k++) a.fillText(GLYPHS.charAt(k), k * aw, (row + 1) * ah);
      });
    }
    function loop(now) {
      raf = 0;
      if (!visible || document.hidden) return;
      var dt = Math.min(0.1, (now - last) / 1000); last = now;
      draw(now / 1000, dt);
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
      if (mq.addEventListener) mq.addEventListener("change", function () { colours(); buildAtlas(); draw(performance.now() / 1000); });
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

// ---- Features: scroll tour for the features section -------------------------
// One big window stays put; each step opens a panel where it lives in the app
// and dims everything else. Wide screens only; narrow ones keep the cards.
(function () {
  var doc = document;
  var sec = doc.getElementById("features");
  if (!sec || !window.matchMedia || !matchMedia("(min-width: 1000px)").matches || !("IntersectionObserver" in window)) return;
  var TR = (doc.documentElement.lang || "").indexOf("tr") === 0;
  var A = (doc.querySelector('link[rel="stylesheet"][href*="site.css"]') || {}).getAttribute
    ? doc.querySelector('link[rel="stylesheet"][href*="site.css"]').getAttribute("href").replace("assets/site.css", "") : "";
  var IMG = A + "assets/img/";

  // rect: left, top, width (percent of the window); panel images keep their aspect.
  var STEPS = TR ? [
    ["Ctrl K, yaz, çal.", "Kütüphanen ve Spotify tek bir arama kutusunda.", "palette", [30, 5.5, 40]],
    ["Sözler şarkıyla akar.", "Yan panelde ya da tam ekranda, istersen çevirisiyle.", "lyrics", [62, 24, 34]],
    ["Kulaklığına göre ses.", "On bant ekolayzer ve OPRA'dan düzeltme eğrileri.", "equalizer", [22.5, 36, 55]],
    ["Kütüphaneni topla.", "Kopyaları bul, listeleri birleştir, yedekle. Hepsi geri alınabilir.", "tools", [41, 31, 52]],
    ["Albümler boşluksuz.", "2–12 saniyelik geçiş ve üç düzey ses dengeleme.", null, [16.5, 88, 63, 10]],
    ["Telefondan yönet.", "Spotify Connect ile telefonundaki Spotify'da bir cihaz olarak görünür.", null, [63.5, 88, 16, 10]]
  ] : [
    ["Ctrl K, type, play.", "Your library and Spotify in one search box.", "palette", [30, 5.5, 40]],
    ["Lyrics keep time.", "In a side panel or full screen, with an optional translation.", "lyrics", [62, 24, 34]],
    ["Sound tuned to your headphones.", "Ten-band equalizer with correction curves from OPRA.", "equalizer", [22.5, 36, 55]],
    ["Tidy up your library.", "Find duplicates, merge playlists, back up. Every edit can be undone.", "tools", [41, 31, 52]],
    ["Albums play gapless.", "Crossfade from 2 to 12 seconds and three levels of normalization.", null, [16.5, 88, 63, 10]],
    ["Control it from your phone.", "Spotify Connect shows Openify as a device in the Spotify app.", null, [63.5, 88, 16, 10]]
  ];
  var ASPECT = { palette: 530 / 728, lyrics: 560 / 570, equalizer: 370 / 748, tools: 580 / 800 };
  var WIN = 850 / 1360;

  var head = sec.querySelector(".section-head");
  var wrap = sec.querySelector(".wrap");
  var bento = sec.querySelector(".bento"), pts = sec.querySelector(".points");
  if (bento) { bento.hidden = true; bento.style.display = "none"; }
  if (pts) pts.classList.add("tour-points");

  var tour = doc.createElement("div");
  tour.className = "tour";
  var stepsHtml = STEPS.map(function (s, i) {
    return '<li class="tour-step" data-i="' + i + '"><span class="tour-n">0' + (i + 1) + '</span><h3>' + s[0] + "</h3><p>" + s[1] + "</p></li>";
  }).join("");
  var panels = STEPS.map(function (s, i) {
    if (!s[2]) return "";
    var r = s[3], h = r[2] * ASPECT[s[2]] / WIN;
    return '<img class="tour-panel" data-i="' + i + '" src="' + IMG + "feat-" + s[2] + '.webp" alt="" style="left:' + r[0] + "%;top:" + r[1] + "%;width:" + r[2] + "%;height:" + h.toFixed(2) + '%">';
  }).join("");
  tour.innerHTML =
    '<ol class="tour-steps">' + stepsHtml + "</ol>" +
    '<div class="tour-stage"><div class="tour-win"><div class="tour-cam">' +
    '<img class="tour-base" src="' + IMG + 'library-2720.webp" alt="" width="1360" height="850">' +
    panels + '<div class="tour-spot"></div></div></div>' +
    '<div class="tour-dots">' + STEPS.map(function () { return "<i></i>"; }).join("") + "</div></div>";
  wrap.insertBefore(tour, pts || null);

  var spot = tour.querySelector(".tour-spot");
  var cam = tour.querySelector(".tour-cam");
  var rectOf = function (i) {
    var s = STEPS[i], r = s[3];
    var h = s[2] ? r[2] * ASPECT[s[2]] / WIN : r[3];
    return [r[0] - 0.8, r[1] - 1.2, r[2] + 1.6, h + 2.4];
  };
  // Camera: zoom the window so the step's region fills the stage, without
  // blowing the screenshot up much past its native pixels.
  var win = tour.querySelector(".tour-win");
  var NATIVE = { palette: 1456, lyrics: 1140, equalizer: 1496, tools: 1600 };
  var cur = 0;
  var zoomTo = function (i) {
    var r = rectOf(i), st = STEPS[i];
    var SW = win.clientWidth, SH = win.clientHeight, CH = SW * WIN;
    var x = r[0] / 100 * SW, y = r[1] / 100 * CH, w = r[2] / 100 * SW, h = r[3] / 100 * CH;
    var nat = st[2] ? NATIVE[st[2]] / (st[3][2] / 100 * SW) : 2720 / SW;
    var s = Math.max(SH / CH, Math.min(nat * 1.1, 0.94 * SW / w, 0.9 * SH / h));
    var tx = SW / 2 - s * (x + w / 2), ty = SH / 2 - s * (y + h / 2);
    tx = Math.min(0, Math.max(SW - s * SW, tx));
    ty = Math.min(0, Math.max(SH - s * CH, ty));
    cam.style.transform = "translate(" + tx.toFixed(1) + "px," + ty.toFixed(1) + "px) scale(" + s.toFixed(3) + ")";
  };
  addEventListener("resize", function () { zoomTo(cur); });
  var set = function (i) {
    tour.querySelectorAll(".tour-step").forEach(function (el) { el.classList.toggle("on", +el.dataset.i === i); });
    tour.querySelectorAll(".tour-panel").forEach(function (el) { el.classList.toggle("on", +el.dataset.i === i); });
    tour.querySelectorAll(".tour-dots i").forEach(function (el, k) { el.classList.toggle("on", k === i); });
    var r = rectOf(i);
    spot.style.left = r[0] + "%"; spot.style.top = r[1] + "%";
    spot.style.width = r[2] + "%"; spot.style.height = r[3] + "%";
    cur = i; zoomTo(i);
    tour.classList.add("lit");
    tour.classList.toggle("panel-on", !!STEPS[i][2]);
  };
  var io = new IntersectionObserver(function (es) {
    es.forEach(function (e) { if (e.isIntersecting) set(+e.target.dataset.i); });
  }, { rootMargin: "-48% 0px -48% 0px" });
  tour.querySelectorAll(".tour-step").forEach(function (el) { io.observe(el); });
  set(0);
})();
