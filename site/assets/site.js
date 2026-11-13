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
    var ptr = { x: null, t: 0 };
    cv.parentElement.addEventListener("pointermove", function (e) {
      var r = cv.getBoundingClientRect();
      ptr.x = (e.clientX - r.left) / r.width; ptr.t = performance.now();
    });

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
      if (ptr.x !== null) {
        var d = x - ptr.x, fade = Math.max(0, 1 - (performance.now() - ptr.t) / 1600);
        v += 0.75 * Math.exp(-(d * d) / 0.004) * fade * (0.7 + 0.3 * grain);
      }
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

// ---- experimental motion -------------------------------------------
(function () {
  var reduce = window.matchMedia && matchMedia("(prefers-reduced-motion: reduce)").matches;
  var doc = document;

  // 1. The accent phrase of the hero is a level meter: bars rise and fall inside the letters.
  // The letters are drawn into a canvas over the (transparent) text, then the bars are
  // composited into them, every frame.
  var em = doc.querySelector(".hero h1 em");
  if (em && !reduce && !(window.matchMedia && matchMedia("(prefers-color-scheme: light)").matches)) {
    // Move the chrome gradient from the h1 onto its plain text, so the accent glyphs underneath
    // the canvas are truly invisible (background-clip:text on the h1 would paint them too).
    var h1 = em.parentNode;
    Array.prototype.slice.call(h1.childNodes).forEach(function (n) {
      if (n.nodeType === 3 && n.textContent.trim()) {
        var sp = doc.createElement("span"); sp.className = "chrome"; sp.textContent = n.textContent;
        h1.replaceChild(sp, n);
      }
    });
    h1.classList.add("split-chrome");
    var cv = doc.createElement("canvas");
    cv.setAttribute("aria-hidden", "true");
    cv.className = "em-meter";
    var probe = doc.createElement("i");
    probe.className = "em-probe";
    em.appendChild(probe);
    em.appendChild(cv);
    em.classList.add("metered");
    var c = cv.getContext("2d");
    var lv = [], on = true, prev = 0, W = 0, H = 0, dpr = 1, text = em.textContent;
    var pad = 0.18; // canvas bleeds past the box so italics, ascenders and descenders fit
    var fit = function () {
      dpr = Math.min(devicePixelRatio || 1, 2);
      W = em.offsetWidth; H = em.offsetHeight;
      var px = H * pad;
      cv.style.left = -px + "px"; cv.style.top = -px + "px";
      cv.style.width = W + 2 * px + "px"; cv.style.height = H + 2 * px + "px";
      cv.width = Math.round((W + 2 * px) * dpr); cv.height = Math.round((H + 2 * px) * dpr);
      var cs = getComputedStyle(em);
      fontStr = cs.fontStyle + " " + cs.fontWeight + " " + cs.fontSize + " " + cs.fontFamily;
      ls = cs.letterSpacing;
      baseY = probe.getBoundingClientRect().top - em.getBoundingClientRect().top;
    };
    var fontStr = "", ls = "0px", baseY = 0;
    var frame = function (now) {
      requestAnimationFrame(frame);
      if (!on) return;
      var dt = Math.min(0.05, (now - prev) / 1000 || 0.016); prev = now;
      var t = now / 1000;
      if (em.offsetWidth !== W || em.offsetHeight !== H) fit();
      var px = H * pad;
      c.setTransform(dpr, 0, 0, dpr, 0, 0);
      c.globalCompositeOperation = "source-over";
      c.clearRect(0, 0, W + 2 * px, H + 2 * px);
      c.font = fontStr;
      if ("letterSpacing" in c) c.letterSpacing = ls;
      c.textBaseline = "alphabetic";
      c.fillStyle = "#000";
      c.fillText(text, px, px + baseY);
      c.globalCompositeOperation = "source-in";
      var top = px, bottom = px + H;
      var base = c.createLinearGradient(0, 0, W, 0);
      base.addColorStop(0, "#7a3410"); base.addColorStop(1, "#5e260b");
      c.fillStyle = base; c.fillRect(0, 0, W + 2 * px, H + 2 * px);
      var hot = c.createLinearGradient(0, bottom, 0, top);
      hot.addColorStop(0, "#e0703a"); hot.addColorStop(0.5, "#f59e5b"); hot.addColorStop(1, "#ffe6cf");
      c.fillStyle = hot;
      c.globalCompositeOperation = "source-atop";
      var bw = 4, gap = 1.25, n = Math.ceil((W + 2 * px) / bw);
      var beat = Math.pow(Math.max(0, Math.sin(t * Math.PI * 2 * 1.9)), 4);
      var k = 1 - Math.exp(-dt * 9);
      for (var i = 0; i < n; i++) {
        var v = 0.58 + 0.17 * Math.sin(t * 1.7 + i * 0.21) + 0.13 * Math.sin(t * 2.9 - i * 0.47) + 0.07 * Math.sin(t * 5.3 + i * 0.9) + 0.16 * beat * (0.5 + 0.5 * Math.sin(i * 0.33 + t));
        lv[i] = lv[i] == null ? v : lv[i] + (v - lv[i]) * k;
        var bh = Math.max(0.1, Math.min(1.05, lv[i])) * (bottom - top);
        c.fillRect(i * bw, bottom - bh, bw - gap, bh + px);
      }
    };
    if ("IntersectionObserver" in window) {
      new IntersectionObserver(function (e) { on = e[0].isIntersecting; }).observe(em);
    }
    if (doc.fonts && doc.fonts.ready) doc.fonts.ready.then(function () { W = 0; });
    requestAnimationFrame(frame);
  }

  // 2. Section headlines arrive word by word, out of a blur.
  var heads = doc.querySelectorAll(".section-head h2, h2.display, .repo-card h2");
  if (!reduce && "IntersectionObserver" in window) {
    heads.forEach(function (h) {
      var i = 0;
      var walk = function (node) {
        Array.prototype.slice.call(node.childNodes).forEach(function (n) {
          if (n.nodeType === 3) {
            var frag = doc.createDocumentFragment();
            n.textContent.split(/(\s+)/).forEach(function (part) {
              if (!part) return;
              if (/^\s+$/.test(part)) { frag.appendChild(doc.createTextNode(part)); return; }
              var s = doc.createElement("span");
              s.className = "w"; s.style.setProperty("--i", i++); s.textContent = part;
              frag.appendChild(s);
            });
            n.parentNode.replaceChild(frag, n);
          } else if (n.nodeType === 1) {
            walk(n);
          }
        });
      };
      walk(h);
      h.classList.add("words");
    });
    var io = new IntersectionObserver(function (es) {
      es.forEach(function (e) { if (e.isIntersecting) { e.target.classList.add("go"); io.unobserve(e.target); } });
    }, { threshold: 0.4 });
    heads.forEach(function (h) { io.observe(h); });
  }

  // 3. The closing wordmark fills like a level meter as you scroll past it.
  var mark = doc.querySelector(".mark-word");
  if (mark) {
    var upd = function () {
      var r = mark.getBoundingClientRect(), vh = innerHeight;
      var p = Math.max(0, Math.min(1, (vh - r.top) / (vh * 0.5 + r.height)));
      mark.style.setProperty("--fill", (reduce ? 100 : Math.round(p * 1000) / 10) + "%");
    };
    upd();
    addEventListener("scroll", upd, { passive: true });
    addEventListener("resize", upd);
  }
})();

// ---- playable page ------------------------------------------------
// A mini player, a working command palette, live keys and timed lyrics.
// Everything is fake and local: no audio, no network.
(function () {
  var doc = document, root = doc.documentElement;
  if (!doc.querySelector(".hero")) return; // home page only; docs stay quiet
  var TR = (root.lang || "").indexOf("tr") === 0;
  var reduce = window.matchMedia && matchMedia("(prefers-reduced-motion: reduce)").matches;
  var L = TR ? {
    play: "Çal", pause: "Duraklat", next: "Sonraki", prev: "Önceki",
    try_: "Burada da çalışıyor. Dene:", ph: "Şarkı ya da komut ara", none: "Sonuç yok",
    cmds: [["Çal / duraklat", "toggle"], ["Sonraki şarkı", "next"], ["Önceki şarkı", "prev"], ["Karışık çalmayı aç / kapat", "shuffle"], ["Sözlere git", "lyrics"]],
    song: "Şarkı", cmd: "Komut",
    lyr_h: "Sözler <em>tam zamanında.</em>",
    lyr_p: "Senkron sözler şarkıyla birlikte akar, çalan satır öne çıkar. Bir satıra tıklarsan şarkı oraya atlar."
  } : {
    play: "Play", pause: "Pause", next: "Next", prev: "Previous",
    try_: "These work here too. Try", ph: "Search songs or run a command", none: "No results",
    cmds: [["Play / pause", "toggle"], ["Next song", "next"], ["Previous song", "prev"], ["Turn shuffle on / off", "shuffle"], ["Go to lyrics", "lyrics"]],
    song: "Song", cmd: "Command",
    lyr_h: "Lyrics, <em>in time.</em>",
    lyr_p: "Synced lyrics move with the song and the line being sung stays in focus. Click a line and the song jumps there."
  };

  // Original placeholder tracks; each cover is drawn from its two colours.
  var TRACKS = [
    { t: "Low Orbit", a: "Kite Season", c: ["#f59e5b", "#7c2d12"], d: 204 },
    { t: "Paper Lanterns", a: "Mira Vale", c: ["#8fb3ff", "#1e2a5a"], d: 187 },
    { t: "Night Bus", a: "The Quiet Hours", c: ["#d29bff", "#3b1d5e"], d: 231 },
    { t: "Copper Sky", a: "Ondine", c: ["#5fe0b0", "#0f3d33"], d: 176 }
  ];
  var LYRICS = [
    [0, "Streetlights hum a quiet tune"],
    [4.2, "I count the windows, one by one"],
    [8.6, "The night is long, the bus is late"],
    [13, "But every song still finds its way"],
    [17.4, "Turn it up and let the city fade"],
    [21.8, "We're halfway home and wide awake"],
    [26.2, "Halfway home and wide awake"],
    [30.6, ""]
  ];

  var ico = function (p) { return '<svg viewBox="0 0 16 16" aria-hidden="true">' + p + "</svg>"; };
  var I = {
    play: ico('<path d="M5 3.2v9.6L12.6 8z" fill="currentColor"/>'),
    pause: ico('<path d="M4.5 3h2.4v10H4.5zM9.1 3h2.4v10H9.1z" fill="currentColor"/>'),
    next: ico('<path d="M3.5 3.5v9l6.5-4.5zM11 3.5h1.6v9H11z" fill="currentColor"/>'),
    prev: ico('<path d="M12.5 3.5v9L6 8zM3.4 3.5H5v9H3.4z" fill="currentColor"/>'),
    search: ico('<circle cx="7" cy="7" r="4.4" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="M10.4 10.4 13.6 13.6" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>'),
    cmd: ico('<path d="M3 4.5h10M3 8h10M3 11.5h6" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" fill="none"/>')
  };

  var cover = function (tr, size) {
    var cv = doc.createElement("canvas"); cv.width = cv.height = size;
    var c = cv.getContext("2d");
    c.fillStyle = tr.c[1]; c.fillRect(0, 0, size, size);
    var g = c.createRadialGradient(size * 0.3, size * 0.28, 0, size * 0.3, size * 0.28, size * 0.9);
    g.addColorStop(0, tr.c[0]); g.addColorStop(1, "rgba(0,0,0,0)");
    c.fillStyle = g; c.fillRect(0, 0, size, size);
    c.globalAlpha = 0.5; c.strokeStyle = "rgba(255,255,255,.55)"; c.lineWidth = size / 48;
    for (var i = 1; i <= 3; i++) { c.beginPath(); c.arc(size * 0.72, size * 0.74, size * 0.12 * i, 0, Math.PI * 2); c.stroke(); }
    return cv.toDataURL();
  };
  TRACKS.forEach(function (t) { t.img = cover(t, 120); });

  // ---- state
  var S = { i: 0, pos: 12, playing: false, shuffle: false };
  var listeners = [];
  var emit = function () { listeners.forEach(function (f) { f(); }); };
  var track = function () { return TRACKS[S.i]; };
  var act = {
    toggle: function () { S.playing = !S.playing; emit(); },
    next: function () { S.i = S.shuffle ? (S.i + 1 + Math.floor(Math.random() * (TRACKS.length - 1))) % TRACKS.length : (S.i + 1) % TRACKS.length; S.pos = 0; S.playing = true; emit(); },
    prev: function () { if (S.pos > 3) S.pos = 0; else S.i = (S.i + TRACKS.length - 1) % TRACKS.length, S.pos = 0; S.playing = true; emit(); },
    shuffle: function () { S.shuffle = !S.shuffle; emit(); },
    seek: function (p) { S.pos = p; emit(); },
    pick: function (i) { S.i = i; S.pos = 0; S.playing = true; emit(); },
    lyrics: function () { var s = doc.getElementById("lyrics"); if (s) s.scrollIntoView({ behavior: reduce ? "auto" : "smooth", block: "center" }); }
  };
  var fmt = function (s) { s = Math.max(0, Math.floor(s)); return Math.floor(s / 60) + ":" + ("0" + (s % 60)).slice(-2); };

  // ---- mini player
  var mp = doc.createElement("div");
  mp.className = "mini";
  mp.innerHTML =
    '<img class="mini-cover" alt="" width="44" height="44">' +
    '<div class="mini-meta"><b></b><small></small></div>' +
    '<div class="mini-ctl">' +
    '<button type="button" data-a="prev" aria-label="' + L.prev + '">' + I.prev + "</button>" +
    '<button type="button" data-a="toggle" class="mini-play"></button>' +
    '<button type="button" data-a="next" aria-label="' + L.next + '">' + I.next + "</button>" +
    "</div>" +
    '<div class="mini-eq" aria-hidden="true"><i></i><i></i><i></i><i></i></div>' +
    '<div class="mini-bar" role="slider" tabindex="0" aria-label="Seek"><span></span></div>';
  doc.body.appendChild(mp);
  mp.addEventListener("click", function (e) {
    var b = e.target.closest("button[data-a]"); if (b) act[b.dataset.a]();
  });
  var bar = mp.querySelector(".mini-bar");
  bar.addEventListener("click", function (e) {
    var r = bar.getBoundingClientRect(); act.seek(track().d * (e.clientX - r.left) / r.width);
  });
  var renderPlayer = function () {
    var t = track();
    mp.querySelector(".mini-cover").src = t.img;
    mp.querySelector("b").textContent = t.t;
    mp.querySelector("small").textContent = t.a;
    var pb = mp.querySelector(".mini-play");
    pb.innerHTML = S.playing ? I.pause : I.play;
    pb.setAttribute("aria-label", S.playing ? L.pause : L.play);
    mp.classList.toggle("on", S.playing);
    mp.classList.toggle("shuf", S.shuffle);
    root.style.setProperty("--cover", t.c[0]);
    root.style.setProperty("--cover-deep", t.c[1]);
  };
  listeners.push(renderPlayer);

  // Show the player once the hero is out of view.
  var hero = doc.querySelector(".hero");
  if (hero && "IntersectionObserver" in window) {
    // ...and step aside while the footer is on screen so it never covers text.
    var pastHero = false, atFoot = false, foot = doc.querySelector(".foot");
    var sync = function () { mp.classList.toggle("shown", pastHero && !atFoot); };
    new IntersectionObserver(function (e) { pastHero = !e[0].isIntersecting; sync(); }, { rootMargin: "-40% 0px 0px 0px" }).observe(hero);
    if (foot) new IntersectionObserver(function (e) { atFoot = e[0].isIntersecting; sync(); }).observe(foot);
  } else mp.classList.add("shown");

  // ---- clock
  var prevT = 0, dirty = true, lyrOn = true, barEl = mp.querySelector(".mini-bar span");
  listeners.push(function () { dirty = true; });
  var tick = function (now) {
    requestAnimationFrame(tick);
    var dt = Math.min(0.1, (now - prevT) / 1000 || 0); prevT = now;
    if (S.playing) {
      S.pos += dt;
      if (S.pos >= track().d) { act.next(); return; }
    }
    if (!S.playing && !dirty) return;
    dirty = false;
    barEl.style.transform = "scaleX(" + (S.pos / track().d).toFixed(4) + ")";
    if (lyrOn) lyricTick();
  };

  // ---- timed lyrics section
  var lyr = null, lines = [], cur = -1;
  var feats = doc.getElementById("features");
  if (feats) {
    lyr = doc.createElement("section");
    lyr.className = "section tight lyrics-sec";
    lyr.id = "lyrics";
    lyr.innerHTML = '<div class="wrap split"><div class="section-head"><h2>' + L.lyr_h + "</h2><p>" + L.lyr_p +
      '</p></div><div class="card lyr-card"><div class="lyr-head"><img alt="" width="40" height="40"><div><b></b><small></small></div><span class="lyr-time"></span></div><div class="lyr-view"><ol class="lyr"></ol></div></div></div>';
    feats.parentNode.insertBefore(lyr, feats.nextSibling);
    if ("IntersectionObserver" in window) new IntersectionObserver(function (e) { lyrOn = e[0].isIntersecting; if (lyrOn) dirty = true; }).observe(lyr);
    var ol = lyr.querySelector(".lyr");
    LYRICS.forEach(function (l, i) {
      var li = doc.createElement("li");
      li.textContent = l[1] || "♪";
      li.addEventListener("click", function () { S.pos = l[0] + 0.01; if (!S.playing) act.toggle(); else emit(); });
      ol.appendChild(li); lines.push(li);
    });
    listeners.push(function () {
      var t = track();
      lyr.querySelector(".lyr-head img").src = t.img;
      lyr.querySelector(".lyr-head b").textContent = t.t;
      lyr.querySelector(".lyr-head small").textContent = t.a;
    });
  }
  var timeEl = lyr && lyr.querySelector(".lyr-time");
  var lyricTick = function () {
    if (!lyr) return;
    var span = LYRICS[LYRICS.length - 1][0] + 4.4, p = S.pos % span, k = 0;
    for (var i = 0; i < LYRICS.length; i++) if (p >= LYRICS[i][0]) k = i;
    var tt = fmt(S.pos) + " / " + fmt(track().d);
    if (timeEl.textContent !== tt) timeEl.textContent = tt;
    if (k !== cur) {
      cur = k;
      lines.forEach(function (li, i) { li.classList.toggle("now", i === k); li.classList.toggle("past", i < k); });
      var ol = lyr.querySelector(".lyr"), li = lines[k];
      ol.style.transform = "translateY(" + (-(li.offsetTop - ol.firstChild.offsetTop)) + "px)";
    }
    var nxt = LYRICS[k + 1] ? LYRICS[k + 1][0] : span;
    lines[k].style.setProperty("--p", Math.min(1, (p - LYRICS[k][0]) / (nxt - LYRICS[k][0])).toFixed(3));
  };

  // ---- command palette
  var pal = doc.createElement("div");
  pal.className = "pal";
  pal.hidden = true;
  pal.innerHTML = '<div class="pal-box" role="dialog" aria-modal="true" aria-label="Command palette"><label class="pal-in">' + I.search +
    '<input type="text" spellcheck="false" autocomplete="off" placeholder="' + L.ph + '"><kbd>Esc</kbd></label><ul class="pal-list" role="listbox"></ul></div>';
  doc.body.appendChild(pal);
  var pin = pal.querySelector("input"), plist = pal.querySelector(".pal-list"), sel = 0, items = [], lastFocus = null;
  var palRender = function () {
    var q = pin.value.trim().toLowerCase();
    items = [];
    TRACKS.forEach(function (t, i) { if (!q || (t.t + " " + t.a).toLowerCase().indexOf(q) >= 0) items.push({ k: "song", i: i, t: t.t, s: t.a, img: t.img }); });
    L.cmds.forEach(function (c) { if (!q || c[0].toLowerCase().indexOf(q) >= 0) items.push({ k: "cmd", a: c[1], t: c[0] }); });
    sel = Math.min(sel, Math.max(0, items.length - 1));
    plist.innerHTML = items.length ? items.map(function (it, n) {
      var lead = it.k === "song" ? '<img alt="" src="' + it.img + '">' : '<span class="pal-ico">' + I.cmd + "</span>";
      return '<li role="option" data-n="' + n + '"' + (n === sel ? ' aria-selected="true"' : "") + ">" + lead +
        '<span class="pal-t"><b>' + it.t + "</b>" + (it.s ? "<small>" + it.s + "</small>" : "") + '</span><em>' + (it.k === "song" ? L.song : L.cmd) + "</em></li>";
    }).join("") : '<li class="pal-none">' + L.none + "</li>";
  };
  var palRun = function (it) {
    if (!it) return;
    palClose();
    if (it.k === "song") act.pick(it.i); else act[it.a]();
  };
  var palOpen = function () {
    if (!pal.hidden) return;
    lastFocus = doc.activeElement;
    pin.value = ""; sel = 0; palRender();
    pal.hidden = false;
    requestAnimationFrame(function () { pal.classList.add("open"); pin.focus(); });
  };
  var palClose = function () {
    pal.classList.remove("open");
    setTimeout(function () { pal.hidden = true; }, reduce ? 0 : 160);
    if (lastFocus && lastFocus.focus) lastFocus.focus();
  };
  pin.addEventListener("input", function () { sel = 0; palRender(); });
  pin.addEventListener("keydown", function (e) {
    if (e.key === "ArrowDown") { sel = (sel + 1) % Math.max(1, items.length); palRender(); e.preventDefault(); }
    else if (e.key === "ArrowUp") { sel = (sel + items.length - 1) % Math.max(1, items.length); palRender(); e.preventDefault(); }
    else if (e.key === "Enter") { palRun(items[sel]); e.preventDefault(); }
    else if (e.key === "Escape") { palClose(); e.preventDefault(); }
  });
  plist.addEventListener("click", function (e) { var li = e.target.closest("li[data-n]"); if (li) palRun(items[+li.dataset.n]); });
  pal.addEventListener("mousedown", function (e) { if (e.target === pal) palClose(); });

  // ---- live keys: every <kbd> on the page lights when its key is held
  var label = function (e) {
    var k = e.key;
    if (k === " ") return "Space";
    if (k === "Control") return "Ctrl";
    if (k === "ArrowRight") return "→";
    if (k === "ArrowLeft") return "←";
    if (k === "ArrowUp") return "↑";
    if (k === "ArrowDown") return "↓";
    if (k === "Escape") return "Esc";
    if (k.length === 1) return k.toUpperCase();
    return k;
  };
  var kbds = function (lab) {
    return Array.prototype.filter.call(doc.querySelectorAll("kbd"), function (k) { return k.textContent.trim() === lab; });
  };
  var combos = [];
  doc.querySelectorAll(".keymap .row").forEach(function (row) {
    var cur = [];
    row.querySelector("dd").childNodes.forEach(function (n) {
      if (n.nodeType === 1 && n.tagName === "KBD") cur.push(n.textContent.trim());
      else if (cur.length) { combos.push({ row: row, keys: cur }); cur = []; }
    });
    if (cur.length) combos.push({ row: row, keys: cur });
  });
  var keysSec = doc.getElementById("keys"), keysVisible = false;
  if (keysSec && "IntersectionObserver" in window) new IntersectionObserver(function (e) { keysVisible = e[0].isIntersecting; }).observe(keysSec);
  var hint = keysSec && keysSec.querySelector(".section-head");
  if (hint) {
    var p = doc.createElement("p");
    p.className = "try";
    p.innerHTML = L.try_ + " <kbd>Ctrl</kbd><kbd>K</kbd> · <kbd>Space</kbd> · <kbd>Ctrl</kbd><kbd>→</kbd>";
    hint.appendChild(p);
  }
  var typing = function (el) { return el && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName)); };
  addEventListener("keydown", function (e) {
    var lab = label(e);
    kbds(lab).forEach(function (k) { k.classList.add("down"); });
    if (e.ctrlKey) kbds("Ctrl").forEach(function (k) { k.classList.add("down"); });
    if (!pal.hidden || typing(e.target)) return;
    // Fire the row whose combination is held right now
    var held = [];
    if (e.ctrlKey) held.push("Ctrl");
    if (e.shiftKey) held.push("Shift");
    if (e.altKey) held.push("Alt");
    if (["Ctrl", "Shift", "Alt", "Meta"].indexOf(lab) < 0) held.push(lab);
    combos.forEach(function (c) {
      if (c.keys.length === held.length && c.keys.every(function (k) { return held.indexOf(k) >= 0; })) {
        c.row.classList.remove("hit"); void c.row.offsetWidth; c.row.classList.add("hit");
      }
    });
    var combo = held.join("+");
    var map = { "Ctrl+K": palOpen, "Ctrl+→": act.next, "Ctrl+←": act.prev, "Alt+S": act.shuffle };
    if (map[combo]) { e.preventDefault(); map[combo](); }
    else if (combo === "Space" && (keysVisible || mp.matches(":hover") || (lyr && lyr.matches(":hover")))) { e.preventDefault(); act.toggle(); }
  });
  addEventListener("keyup", function (e) {
    kbds(label(e)).forEach(function (k) { k.classList.remove("down"); });
    if (!e.ctrlKey) kbds("Ctrl").forEach(function (k) { k.classList.remove("down"); });
  });
  addEventListener("blur", function () { doc.querySelectorAll("kbd.down").forEach(function (k) { k.classList.remove("down"); }); });

  emit();
  requestAnimationFrame(tick);
})();
