/* ============================================================================
   Cinefly Presentation — Autopilot playback controller (autopilot.js)

   Press the corner button and the deck runs itself: each slide's narration
   clip plays with a 1 s buffer before and after, then the deck advances,
   to the end. Manual navigation always takes over (state → paused).

   Integration contract (see AUTOPILOT-PLAN.md):
   · deck.js is FROZEN and exposes no public API — we advance by synthesising
     a `keydown` ArrowRight on document (deck.js's own listener handles it,
     single-flight lock included) and observe position via #deck-counter text
     and #overview state. Nothing in deck.js is modified.
   · Data: window.AUTOPILOT_DATA from autopilot-data.js. With an empty slides
     array this file renders nothing — the deck is unchanged.

   How one slide's timing is decided (every number comes from that one table):
     advance  the slide transition itself (~700 ms) before anything starts
     lead     silence after the slide is on screen, before its clip starts
     clip     plays to its natural end — `ended` is the precise trigger
     dur      expected clip length — the fallback contract for this slide.
              If `ended` never arrives the watchdog fires at dur + guard; the
              first alarm grants one cushion window so a rounded duration can
              never clip real audio, the second advances regardless. If the
              clip cannot play at all, the deck holds for dur rather than the
              generic fallback. Changing dur therefore changes the switching
              behaviour on exactly this slide — keep it true to the file.
     tail     silence after the clip ends, before the deck advances
   ========================================================================== */
(function () {
  'use strict';

  var DATA = window.AUTOPILOT_DATA;
  if (!DATA || !Array.isArray(DATA.slides) || DATA.slides.length === 0) return;

  var B = Object.assign(
    { advance: 700, lead: 1000, tail: 1000, fallback: 8000, guard: 2500 },
    DATA.buffer || {}
  );

  var stage = document.getElementById('deck-stage');
  var counter = document.getElementById('deck-counter');
  var overview = document.getElementById('overview');
  if (!stage || !counter) return;

  var slides = Array.prototype.slice.call(stage.querySelectorAll(':scope > section.slide'));
  var total = slides.length;
  var byId = {};
  DATA.slides.forEach(function (s) { byId[s.id] = s; });
  var clips = slides.map(function (s) { return byId[s.getAttribute('data-id')] || null; });
  if (!clips.some(Boolean)) return;                    // no matching clips → stay silent

  var stubs = DATA.slides.filter(function (s) { return s.stub; }).map(function (s) { return s.id; });
  var speechSeconds = DATA.slides.reduce(function (a, s) { return a + (Number(s.dur) || 0); }, 0);
  var runSeconds = speechSeconds + (clips.length * (B.lead + B.tail + B.advance)) / 1000;

  /* ---------------- button ---------------- */

  var btn = document.createElement('button');
  btn.type = 'button';
  btn.className = 'ap-btn';
  btn.innerHTML =
    '<svg class="ic ic-play" viewBox="0 0 24 24" aria-hidden="true"><path d="M8 5v14l11-7z"/></svg>' +
    '<svg class="ic ic-pause" viewBox="0 0 24 24" aria-hidden="true"><path d="M7 5h4v14H7zM13 5h4v14h-4z"/></svg>' +
    '<svg class="ic ic-replay" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 5V2L7 6l5 4V7c3.31 0 6 2.69 6 6s-2.69 6-6 6-6-2.69-6-6H4c0 4.42 3.58 8 8 8s8-3.58 8-8-3.58-8-8-8z"/></svg>';
  document.body.appendChild(btn);

  var LABELS = { idle: 'Play from this slide', playing: 'Pause', paused: 'Resume', done: 'Restart from the top' };
  var ICON   = { idle: 'play', playing: 'pause', paused: 'play', done: 'replay' };

  var state = 'idle';
  var expected = 0;        // slide index we believe is on screen
  var pending = null;      // set while an advance is in flight
  var resolved = false;    // current slide already handed over to the tail/advance
  var watch = null;        // stall watchdog for the current slide
  var timers = [];
  var audio = new Audio();
  audio.preload = 'auto';

  function setState(s) {
    state = s;
    btn.dataset.state = ICON[s];
    btn.title = LABELS[s];
    btn.setAttribute('aria-label', LABELS[s]);
    try {
      window.__autopilot = {
        state: s,
        pos: currentIndex(),
        total: total,
        stubs: stubs,
        speech: Math.round(speechSeconds * 10) / 10,
        runtime: Math.round(runSeconds),
      };
    } catch (e) { /* noop */ }
  }

  function later(fn, ms) {
    var t = setTimeout(fn, ms);
    timers.push(t);
    return t;
  }
  function clearTimers() {
    timers.forEach(clearTimeout);
    timers = [];
    if (watch) { clearTimeout(watch); watch = null; }
  }

  /* deck.js writes "n / total" into #deck-counter — read it, don't trust ours */
  function currentIndex() {
    var m = (counter.textContent || '').match(/(\d+)\s*\/\s*(\d+)/);
    return m ? Math.max(0, parseInt(m[1], 10) - 1) : 0;
  }
  function overviewOpen() {
    return !!(overview && overview.classList.contains('is-open'));
  }

  /* ---------------- advancing ---------------- */

  function press(key) {
    document.dispatchEvent(new KeyboardEvent('keydown', { key: key, bubbles: true, cancelable: true }));
  }

  function advanceTo(target) {
    pending = { target: target, at: Date.now() };
    press('ArrowRight');
    later(function () {
      pending = null;
      if (state !== 'playing') return;
      var c = currentIndex();
      if (overviewOpen()) { pause(); return; }
      if (c === target) { expected = target; playSlide(target); }
      else pause();                            // didn't land — takeover or locked; yield
    }, B.advance + 590);                        // advance wait + morph(550) + margin
  }

  function goHome() {
    pending = { target: 0, at: Date.now() };
    press('Home');
    later(function () {
      pending = null;
      if (state !== 'playing') return;
      if (currentIndex() === 0) { expected = 0; playSlide(0); }
      else pause();
    }, B.advance + 590);
  }

  /* ---------------- playback ---------------- */

  /* A slide is finished exactly once: natural end, watchdog, or load failure.
     `resolved` is the single-flight guard so the tail buffer can't run twice. */
  function resolveSlide(i, why) {
    if (state !== 'playing' || i !== expected || resolved) return;
    resolved = true;
    if (watch) { clearTimeout(watch); watch = null; }
    if (why !== 'ended') console.warn('[autopilot] slide ' + (clips[i] ? clips[i].id : i) + ': ' + why);
    audio.onended = null;
    audio.onerror = null;
    later(function () {
      if (state !== 'playing') return;
      if (overviewOpen()) { pause(); return; }
      if (i + 1 < total) advanceTo(i + 1);
      else finish();
    }, B.tail);
  }

  /* Watchdog: the declared `dur` is the contract for how long this slide
     should be speaking. If playback never reports `ended`, we advance anyway
     rather than parking the deck forever. The first alarm only grants one
     cushion window — encoder durations round a hair under real playback, and
     a truncated slide is worse than a late one. The second alarm is final. */
  function armWatch(i, clip) {
    var expectMs = (Number(clip.dur) || 0) * 1000;
    if (expectMs <= 0) return;
    var alarms = 0;
    var tick = function () {
      watch = null;
      if (state !== 'playing' || i !== expected || resolved) return;
      if (audio.ended) { resolveSlide(i, 'ended'); return; }
      if (audio.paused || audio.error) { resolveSlide(i, 'stalled'); return; }
      if (++alarms < 2) { watch = setTimeout(tick, B.guard); return; }
      resolveSlide(i, 'watchdog (past dur ' + clip.dur + 's + guard)');
    };
    watch = setTimeout(tick, expectMs + B.guard);
  }

  function playSlide(i) {
    setState('playing');
    expected = i;
    resolved = false;
    var clip = clips[i];
    later(function () {
      if (state !== 'playing') return;
      if (currentIndex() !== i || overviewOpen()) { pause(); return; }   // took over during lead
      if (!clip) { holdAndAdvance(i, B.fallback); return; }
      // preload the next clip while this one plays
      var nx = clips[i + 1];
      if (nx) { var a2 = new Audio(nx.src); a2.preload = 'auto'; a2.load(); }

      var expectMs = (Number(clip.dur) || 0) * 1000;
      audio.src = clip.src;
      audio.currentTime = 0;
      audio.onended = null;
      audio.onerror = null;

      var p = audio.play();
      if (p && p.catch) p.catch(function (err) {
        if (state !== 'playing') return;
        console.warn('[autopilot] clip failed to start: ' + clip.id, err);
        // hold for the clip's own length when we know it, else the generic fallback
        holdAndAdvance(i, expectMs > 0 ? expectMs + B.tail : B.fallback);
      });

      audio.onended = function () { resolveSlide(i, 'ended'); };
      audio.onerror = function () {
        if (state !== 'playing') return;
        holdAndAdvance(i, expectMs > 0 ? expectMs + B.tail : B.fallback);
      };

      armWatch(i, clip);
    }, B.lead);
  }

  function holdAndAdvance(i, ms) {
    if (state !== 'playing' || i !== expected || resolved) return;
    resolved = true;
    if (watch) { clearTimeout(watch); watch = null; }
    audio.onended = null;
    audio.onerror = null;
    later(function () {
      if (state !== 'playing') return;
      if (i + 1 < total) advanceTo(i + 1);
      else finish();
    }, ms);
  }

  function pause() {
    clearTimers();
    audio.pause();
    audio.onended = null;
    audio.onerror = null;
    pending = null;
    resolved = false;
    expected = currentIndex();
    setState('paused');
  }

  function finish() {
    clearTimers();
    audio.pause();
    audio.onended = null;
    audio.onerror = null;
    pending = null;
    resolved = false;
    setState('done');
  }

  function startRun(from) {
    clearTimers();
    audio.pause();
    audio.onended = null;
    audio.onerror = null;
    resolved = false;
    if (from === 0 && currentIndex() === 0) { playSlide(0); return; }
    if (from === 0) { goHome(); return; }                 // restart from top
    playSlide(from);
  }

  function resume() {
    clearTimers();
    resolved = false;
    playSlide(currentIndex());
  }

  /* ---------------- input ---------------- */

  btn.addEventListener('click', function () {
    if (state === 'idle') startRun(currentIndex());
    else if (state === 'playing') pause();
    else if (state === 'paused') resume();
    else if (state === 'done') startRun(0);
  });

  /* Space toggles autopilot while active; passes through untouched while idle.
     Capture phase on document runs before deck.js's bubble listener, and
     stopPropagation() prevents delivery to it entirely. */
  document.addEventListener('keydown', function (e) {
    if (e.code !== 'Space' && e.key !== ' ') return;
    if (state === 'idle') return;
    e.preventDefault();
    e.stopPropagation();
    if (state === 'playing') pause();
    else if (state === 'paused') resume();
    else if (state === 'done') startRun(0);
  }, true);

  /* Takeover poller: user navigation (arrows/dots/Home/End) or overview
     while we're playing → yield to paused. While an advance is in flight
     the poller stands down (advanceTo resolves the expectation). */
  setInterval(function () {
    if (state !== 'playing' && state !== 'paused') return;
    if (pending) return;
    if (overviewOpen()) { pause(); return; }
    var c = currentIndex();
    if (c !== expected) {
      if (state === 'playing') pause();
      else expected = c;                       // moved while paused — track, stay paused
    }
  }, 300);

  setState('idle');
})();
