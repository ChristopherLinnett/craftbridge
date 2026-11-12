/* ============================================================================
   Cinefly Presentation — Deck navigation state machine (deck.js)
   Shared runtime from deck-kit (single source: dev/cinefly/deck-kit/runtime/).
   Each deck's build installs it; never edit an installed copy.

   Reads slides from the DOM (#deck-stage > section.slide[data-id][data-type]).
   Input:  → / Space / PageDown · ← / PageUp · Home / End · Esc (overview) · F (fullscreen)
           dot click · #<id> hash deep-link on load
   Motion: ~450ms (--slide-ms) --ease, opacity + transform only.
           Special case: a slide carrying data-morph-to="<next id>" runs a
           ~550ms hero→live FLIP expansion into that slide (see
           startHeroExpansion) instead of the standard push.
   Lifecycle: data-type="demo" → iframe mounted on enter / unmounted on exit.
   ========================================================================== */
(function () {
  'use strict';

  var DESIGN_W = 1280, DESIGN_H = 720;
  var TRANSITION_MS = 450;             // keep in sync with --slide-ms
  var MORPH_MS = 550;                  // hero-expansion duration (data-morph-to)
  var MORPH_EASE = 'cubic-bezier(0.22, 1, 0.36, 1)';   // same as --ease

  var stage = document.getElementById('deck-stage');
  var viewport = document.getElementById('deck-viewport');
  var counter = document.getElementById('deck-counter');
  var rail = document.getElementById('dot-rail');
  var overview = document.getElementById('overview');
  var overviewGrid = overview ? overview.querySelector('.overview-grid') : null;

  var slides = Array.prototype.slice.call(stage.querySelectorAll(':scope > section.slide'));
  var total = slides.length;
  var index = 0;
  var mode = 'deck';                   // 'deck' | 'overview'
  var animating = false;               // single-flight lock

  /* ---------------- helpers ---------------- */

  function slideTitle(s) {
    var t = s.querySelector('.slide-title') || s.querySelector('.divider-title') || s.querySelector('.eyebrow');
    return t ? t.textContent.trim() : '';
  }

  /* Fit the fixed 1280×720 stage to the viewport (same as demo.html's fit()). */
  function fit() {
    var s = Math.min(window.innerWidth / DESIGN_W, window.innerHeight / DESIGN_H);
    stage.style.transform = 'scale(' + s + ')';
    stage.style.left = ((window.innerWidth - DESIGN_W * s) / 2) + 'px';
    stage.style.top = ((window.innerHeight - DESIGN_H * s) / 2) + 'px';
  }

  /* ---------------- demo lifecycle hooks ---------------- */

  function mountDemo(slide) {
    if (!slide) return;
    var slots = slide.querySelectorAll('.demo-slot[data-demo-src]');
    Array.prototype.forEach.call(slots, function (slot) {
      if (slot.querySelector('iframe')) return;
      var f = document.createElement('iframe');
      f.src = slot.getAttribute('data-demo-src');
      f.setAttribute('frameborder', '0');
      f.setAttribute('allow', 'autoplay');
      f.setAttribute('tabindex', '-1');
      f.setAttribute('aria-hidden', 'true');
      slot.appendChild(f);
    });
  }

  function unmountDemo(slide) {
    if (!slide) return;
    var slots = slide.querySelectorAll('.demo-slot');
    Array.prototype.forEach.call(slots, function (slot) { slot.innerHTML = ''; });
  }

  /* ---------------- chrome ---------------- */

  function setAria() {
    slides.forEach(function (s, i) {
      s.setAttribute('aria-hidden', i === index ? 'false' : 'true');
    });
  }

  function buildRail() {
    if (!rail) return;
    rail.innerHTML = '';
    var prevGroup = null;
    slides.forEach(function (s, i) {
      var id = s.getAttribute('data-id') || String(i);
      var group = /^[A-Za-z]/.test(id) ? id.charAt(0) : '0';
      if (prevGroup !== null && group !== prevGroup) {
        var gap = document.createElement('span');
        gap.className = 'dot-gap';
        gap.setAttribute('aria-hidden', 'true');
        rail.appendChild(gap);
      }
      prevGroup = group;
      var d = document.createElement('button');
      d.type = 'button';
      d.className = 'dot';
      d.title = id + ' · ' + slideTitle(s);
      d.setAttribute('aria-label', 'Go to slide ' + id);
      d.addEventListener('click', function () { goTo(i); });
      rail.appendChild(d);
    });
  }

  function updateChrome() {
    if (counter) counter.textContent = (index + 1) + ' / ' + total;
    if (!rail) return;
    var dots = rail.querySelectorAll('.dot');
    var di = 0;
    for (var i = 0; i < dots.length; i++) {
      dots[i].classList.toggle('is-active', di === index);
      di++;
    }
  }

  function setHash(slide) {
    var id = slide && slide.getAttribute('data-id');
    if (!id) return;
    try { history.replaceState(null, '', '#' + id); } catch (e) { /* file:// */ }
  }

  /* ---------------- transitions ---------------- */

  function clearTransitionClasses(s) {
    s.classList.remove('is-enter-left', 'is-enter-right', 'is-exit-left', 'is-exit-right');
  }

  /* ---------------- hero→live expansion morph (data-morph-to) ---------------- */

  /** True when the current slide declares the next one as its morph target. */
  function isHeroMorph(cur, nxt) {
    return !!cur && !!nxt
      && !!cur.getAttribute('data-morph-to')
      && cur.getAttribute('data-morph-to') === nxt.getAttribute('data-id');
  }

  /** Remove every inline style the morph applies (restore CSS-class defaults). */
  function clearMorphStyles(cur, nxt) {
    if (cur) { cur.style.transition = ''; cur.style.opacity = ''; }
    if (nxt) {
      nxt.style.transition = ''; nxt.style.opacity = '';
      nxt.style.background = ''; nxt.style.zIndex = '';
      var slot = nxt.querySelector('.demo-slot');
      if (slot) { slot.style.transition = ''; slot.style.transform = ''; slot.style.transformOrigin = ''; }
    }
  }

  /**
   * FLIP expansion: the D2 demo slot starts parked at D1's framed-hero rect
   * (~60% size, off-centre) and grows out to full-bleed over MORPH_MS with the
   * deck's easing. D1 fades out in place underneath so the hero reads as
   * "becoming" the demo. The demo iframe is mounted by goTo before this runs,
   * so it loads during the expansion and the demo's own 500ms focus delay lands
   * the typing right as the expansion completes.
   *
   * Returns false (applying nothing) when the geometry can't be computed
   * cleanly — the caller then falls back to the standard push transition, so
   * the slide can never break.
   */
  function startHeroExpansion(cur, nxt) {
    var hero = cur.querySelector('.mockup-frame');
    var slot = nxt.querySelector('.demo-slot');
    if (!hero || !slot) return false;

    var hr = hero.getBoundingClientRect();
    var fr = slot.getBoundingClientRect();
    var layoutW = slot.offsetWidth;
    if (!(hr.width > 8) || !(hr.height > 8) || !(fr.width > 8) || !(fr.height > 8) || !(layoutW > 0)) {
      return false;
    }

    // The stage is uniformly scaled by fit(); viewport-space deltas must be
    // divided back into the slot's local (design-space) coordinates. Scale
    // ratios themselves are scale-invariant.
    var s = fr.width / layoutW;
    if (!(s > 0.01)) return false;
    var sx = hr.width / fr.width;
    var sy = hr.height / fr.height;
    if (!isFinite(sx) || !isFinite(sy) || sx <= 0.05 || sy <= 0.05) return false;
    var dx = (hr.left - fr.left) / s;
    var dy = (hr.top - fr.top) / s;

    try {
      // D2: on-stage immediately, transparent so D1 shows through while fading.
      clearTransitionClasses(nxt);
      nxt.style.transition = 'none';
      nxt.style.background = 'transparent';
      nxt.style.opacity = '1';
      nxt.style.zIndex = '3';

      // FLIP: park the demo slot at the hero rect (local space), flush, grow.
      slot.style.transition = 'none';
      slot.style.transformOrigin = '0 0';
      slot.style.transform = 'translate(' + dx + 'px, ' + dy + 'px) scale(' + sx + ', ' + sy + ')';
      void nxt.offsetWidth;                       // force style flush

      slot.style.transition = 'transform ' + MORPH_MS + 'ms ' + MORPH_EASE;
      slot.style.transform = 'translate(0px, 0px) scale(1, 1)';

      // D1: fade out in place (no slide) under the growing demo.
      cur.classList.remove('is-active');
      cur.style.transition = 'opacity ' + MORPH_MS + 'ms ' + MORPH_EASE;
      cur.style.opacity = '0';

      nxt.classList.add('is-active');
      return true;
    } catch (e) {
      clearMorphStyles(cur, nxt);
      return false;
    }
  }

  /** Animated go-to. No-op while animating (single-flight lock). */
  function goTo(n) {
    if (mode === 'overview') exitOverview();
    n = Math.max(0, Math.min(total - 1, n));
    if (n === index || animating || total === 0) return;

    var dir = n > index ? 1 : -1;
    var cur = slides[index];
    var nxt = slides[n];
    animating = true;

    unmountDemo(cur);
    mountDemo(nxt);

    // data-morph-to: hero→live expansion when the geometry allows; else standard push.
    var morphed = isHeroMorph(cur, nxt) && startHeroExpansion(cur, nxt);

    if (!morphed) {
      // Park the target off-stage in the direction of travel, flush, then run.
      clearTransitionClasses(nxt);
      nxt.classList.add(dir > 0 ? 'is-enter-right' : 'is-enter-left');
      void nxt.offsetWidth;                       // force style flush

      cur.classList.remove('is-active');
      cur.classList.add(dir > 0 ? 'is-exit-left' : 'is-exit-right');
      nxt.classList.remove('is-enter-right', 'is-enter-left');
      nxt.classList.add('is-active');
    }

    index = n;
    updateChrome();
    setAria();
    setHash(nxt);

    window.setTimeout(function () {
      if (morphed) clearMorphStyles(cur, nxt);
      cur.classList.remove('is-exit-left', 'is-exit-right');
      animating = false;
    }, (morphed ? MORPH_MS : TRANSITION_MS) + 40);
  }

  /** Instant jump (init / hash load) — no transition. */
  function jumpTo(n) {
    n = Math.max(0, Math.min(total - 1, n));
    slides.forEach(function (s, i) {
      clearTransitionClasses(s);
      s.classList.toggle('is-active', i === n);
    });
    slides.forEach(unmountDemo);
    index = n;
    mountDemo(slides[n]);
    updateChrome();
    setAria();
    setHash(slides[n]);
  }

  /* ---------------- overview grid ---------------- */

  function scaleOverviewTiles() {
    if (!overviewGrid) return;
    var inners = overviewGrid.querySelectorAll('.ov-inner');
    Array.prototype.forEach.call(inners, function (inner) {
      var w = inner.parentElement.clientWidth;
      inner.style.transform = 'scale(' + (w / DESIGN_W) + ')';
    });
  }

  function buildOverview() {
    if (!overviewGrid) return;
    overviewGrid.innerHTML = '';
    slides.forEach(function (s, i) {
      var tile = document.createElement('div');
      tile.className = 'overview-tile';

      var inner = document.createElement('div');
      inner.className = 'ov-inner';
      var clone = s.cloneNode(true);
      clone.classList.add('ov-slide');
      clearTransitionClasses(clone);
      clone.setAttribute('aria-hidden', 'false');
      // Never let a live demo iframe animate inside the grid.
      Array.prototype.forEach.call(clone.querySelectorAll('.demo-slot iframe'), function (f) { f.remove(); });
      inner.appendChild(clone);
      tile.appendChild(inner);

      var label = document.createElement('div');
      label.className = 'overview-label';
      label.textContent = (s.getAttribute('data-id') || '?') + ' · ' + slideTitle(s);
      tile.appendChild(label);

      tile.addEventListener('click', function () {
        exitOverview();
        goTo(i);
      });
      overviewGrid.appendChild(tile);
    });
    scaleOverviewTiles();
  }

  function enterOverview() {
    if (mode === 'overview' || animating) return;
    mode = 'overview';
    buildOverview();
    overview.classList.add('is-open');
    overview.setAttribute('aria-hidden', 'false');
    // Tiles must be measured AFTER the grid is visible — clientWidth is 0 while
    // display:none, which would scale every clone to zero.
    scaleOverviewTiles();
  }

  function exitOverview() {
    if (mode !== 'overview') return;
    mode = 'deck';
    overview.classList.remove('is-open');
    overview.setAttribute('aria-hidden', 'true');
    if (overviewGrid) overviewGrid.innerHTML = '';
  }

  function toggleOverview() {
    if (mode === 'overview') exitOverview(); else enterOverview();
  }

  /* ---------------- input map ---------------- */

  document.addEventListener('keydown', function (e) {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    switch (e.key) {
      case 'ArrowRight':
      case ' ':
      case 'PageDown':
        if (mode === 'overview') return;
        e.preventDefault(); goTo(index + 1); break;
      case 'ArrowLeft':
      case 'PageUp':
        if (mode === 'overview') return;
        e.preventDefault(); goTo(index - 1); break;
      case 'Home':
        e.preventDefault(); goTo(0); break;
      case 'End':
        e.preventDefault(); goTo(total - 1); break;
      case 'Escape':
        e.preventDefault(); toggleOverview(); break;
      case 'f': case 'F':
        if (document.fullscreenElement) { document.exitFullscreen(); }
        else if (document.documentElement.requestFullscreen) { document.documentElement.requestFullscreen(); }
        break;
    }
  });

  window.addEventListener('resize', function () {
    fit();
    if (mode === 'overview') scaleOverviewTiles();
  });

  /* ---------------- init ---------------- */

  fit();
  buildRail();

  var start = 0;
  var hash = (window.location.hash || '').replace(/^#/, '');
  if (hash) {
    for (var i = 0; i < total; i++) {
      if (slides[i].getAttribute('data-id') === hash) { start = i; break; }
    }
  }
  jumpTo(start);
})();
