// Tine Story Mode, scene engine. One place to look: the stage acts out what the dialogue box says.
// Everything shown comes from TINE_SCENES (scenes.js) and TINE_DATA (data.js). Rendering is a pure
// function of the play clock t, so scrubbing, pausing and deep links show the same frame; only sounds
// fire on the way through.
(function () {
  'use strict';
  const S = window.TINE_SCENES, D = window.TINE_DATA;
  const W = 640, H = 272, HUD = 20, GROUND = 236, MX = 172;
  const SPR = 1.7, SLOT = { claude: MX - 58, codex: MX - 112 }, PIPE_X = 272;
  const DAY = 86400000, CPS = 70;
  const PX = '"Press Start 2P", ui-monospace, monospace';
  const parse = s => Date.parse(s.length === 10 ? s + 'T12:00Z' : (/[Z+]/.test(s.slice(10)) ? s : s + 'Z'));
  const clamp = (x, a = 0, b = 1) => Math.max(a, Math.min(b, x));
  const ease = x => x < 0.5 ? 2 * x * x : 1 - Math.pow(-2 * x + 2, 2) / 2;
  const rnd = s => { const x = Math.sin(s * 127.1 + 311.7) * 43758.5453; return x - Math.floor(x); };
  const words = s => (s || '').split(/\s+/).filter(Boolean).length;

  // ── timeline: banner, run, steps ──────────────────────────────────────────────
  // A step lasts as long as its line takes to type and to read; a run between scenes lasts longer the
  // more days it covers, but never long.
  const lineDur = st => st.dur || (st.text ? Math.max(st.text.length / CPS + 0.6, 0.75 + words(st.text) * 0.23) : 1.2);
  const runDur = (a, b) => clamp(0.8 + 0.25 * Math.log2(1 + (b - a) / DAY), 1.0, 1.6);
  const worldById = Object.fromEntries(S.worlds.map(w => [w.id, w]));
  const items = [];
  let T = 0, prev = null, curW = null, cam = 0, lastWorld = null;
  for (const sc of S.scenes) {
    sc.d = parse(sc.at);
    if (prev) { const dur = runDur(prev.d, sc.d); items.push({ type: 'run', t0: T, t1: T + dur, from: prev, to: sc, cam0: cam }); T += dur; cam += W; }
    sc.cam = cam;
    if (sc.skip) { sc.w = lastWorld; items.push({ type: 'skip', t0: T, t1: T + 3.4, scene: sc }); T += 3.4; prev = sc; continue; }
    sc.w = lastWorld = worldById[sc.world];
    if (sc.world !== curW) { curW = sc.world; sc.w.t0 = T; items.push({ type: 'banner', t0: T, t1: T + 2.3, scene: sc }); T += 2.3; }
    sc.t0 = T;
    sc.steps.forEach((st, i) => {
      // a step without its own date keeps the date of the step before it (time never runs backwards)
      st.d = st.at ? parse(st.at) : i ? sc.steps[i - 1].d : sc.d; st.scene = sc;
      const dur = lineDur(st); st.t0 = T; st.t1 = T + dur;
      items.push({ type: 'step', t0: T, t1: T + dur, scene: sc, st, i }); T += dur;
    });
    sc.t1 = T; prev = sc;
  }
  const lastScene = prev;
  const T_STORY = T, T_END = T + 4;
  items.push({ type: 'end', t0: T, t1: T_END, scene: lastScene });
  const scenes = S.scenes;
  const finScene = scenes.find(s => s.final), finRaise = finScene && finScene.steps.find(s => s.act && s.act.startsWith('raise'));
  const steps = items.filter(i => i.type === 'step');
  // seconds since the play clock passed date dd, if that happened in the latest run
  function dToSecs(dd, t) {
    const it = itemAt(t);
    const prevRun = it.type === 'run' ? it : [...items].reverse().find(x => x.type === 'run' && x.t1 <= t);
    if (!prevRun || dd <= prevRun.from.d || dd > prevRun.to.d) return -1;
    let lo = prevRun.t0, hi = prevRun.t1;
    for (let i = 0; i < 30; i++) { const mid = (lo + hi) / 2; if (dateAt(mid, prevRun) < dd) lo = mid; else hi = mid; }
    return t - hi;
  }
  function itemAt(t) { let it = items[0]; for (const x of items) { if (x.t0 <= t) it = x; else break; } return it; }
  const camAt = (t, it) => it.type === 'run' ? it.cam0 + W * ease(clamp((t - it.t0) / (it.t1 - it.t0))) : it.scene.cam;
  function dateAt(t, it) {
    if (it.type === 'run') return it.from.d + (it.to.d - it.from.d) * ease(clamp((t - it.t0) / (it.t1 - it.t0)));
    if (it.type === 'step') return it.st.d;
    if (it.type === 'end') return it.scene.steps[it.scene.steps.length - 1].d;
    return it.scene.d;
  }
  for (const w of S.worlds) { const later = S.worlds.filter(x => x.t0 > w.t0).map(x => x.t0); w.t1 = later.length ? Math.min(...later) : T_STORY; }
  // the skip card ends the world before it
  for (const it of items) if (it.type === 'skip') for (const w of S.worlds) if (w.t0 < it.t0 && w.t1 > it.t0) w.t1 = it.t0;

  // ── lasting state: party, equipment, levels, tower ────────────────────────────
  const gains = steps.filter(i => i.st.gain);
  function stateAt(t) {
    const s = { party: {}, equip: {}, lv: [], calm: null };
    for (const it of gains) {
      if (it.t0 > t) break;
      const g = it.st.gain;
      if (g.party) [].concat(g.party).forEach((k, i) => { s.party[k] = it.t0 + i * 0.5; });
      if (g.lv) s.lv.push({ topic: g.lv, t: it.t0 });
      if (g.calm) s.calm = it.t0;
    }
    return s;
  }
  const loc = [[parse('2026-06-14'), 0]];
  // the share of the tower that is tests (drawn as scaffolding on top of the code bricks)
  const testShare = [[parse('2026-06-14'), 0.4]];
  for (const [date, ref, rust, , test] of D.loc) if (ref !== 'v0.6.987') { loc.push([parse(date), rust]); testShare.push([parse(date), rust ? (test || 0) / rust : 0]); }
  loc.sort((a, b) => a[0] - b[0]);
  const overrides = gains.filter(i => i.st.gain.tower !== undefined).map(i => ({ t: i.t0, d: i.st.d, v: i.st.gain.tower }));
  function interp(pts, d) {
    if (d <= pts[0][0]) return pts[0][1];
    for (let i = 1; i < pts.length; i++) if (d <= pts[i][0]) { const [da, va] = pts[i - 1], [db, vb] = pts[i]; return va + (vb - va) * (d - da) / (db - da || 1); }
    return pts[pts.length - 1][1];
  }
  function towerAt(t) {
    const d = dateAt(t, itemAt(t));
    let ov = null; for (const o of overrides) if (o.t <= t) ov = o;
    let v = interp(ov ? [[ov.d, ov.v], ...loc.filter(p => p[0] > ov.d)] : loc, d);
    if (ov && t - ov.t < 1.4) { const before = towerAt(ov.t - 1e-4); v = before + (v - before) * ease((t - ov.t) / 1.4); }
    return v;
  }
  const TOWER_MAX = 460000, TOWER_PX = 196;
  const models = (S.models || []).map(m => ({ ...m, d: parse(m.at) })).sort((a, b) => a.d - b.d);
  function equipAt(who, d) { let m = null; for (const x of models) if (x.who === who && x.d <= d) m = x; return m; }
  const starRows = D.stars.map(([d, c], i) => ({ d: parse(d), c, n: c - (i ? D.stars[i - 1][1] : 0) }));
  function starsAt(d) {
    let p = 0;
    for (const r of starRows) { const a = r.d - DAY / 2; if (d < a) return p; if (d < a + DAY) return Math.round(p + r.n * (d - a) / DAY); p = r.c; }
    return p;
  }
  const FIN = S.finale;
  // the finale's release counts from its exact time (the data has only its day)
  const rels = D.releases.map(([tag, date]) => ({ tag, d: Date.parse(date) })).filter(r => !/beta/.test(r.tag) && !(FIN && r.tag === FIN.tag))
    .concat(FIN ? [{ tag: FIN.tag, d: parse(FIN.at) }] : []).sort((a, b) => a.d - b.d);
  const finInData = !!FIN && D.releases.some(r => r[0] === FIN.tag);
  function versionAt(d) { let v = null; for (const r of rels) if (r.d <= d) v = r.tag; return v; }

  // ── drawing ─────────────────────────────────────────────────────────────────
  const off = document.createElement('canvas'); off.width = W; off.height = H;
  let c = off.getContext('2d');
  const hex = h => [parseInt(h.slice(1, 3), 16), parseInt(h.slice(3, 5), 16), parseInt(h.slice(5, 7), 16)];
  const mixc = (a, b, f) => { const A = hex(a), B = hex(b); return '#' + A.map((x, i) => Math.round(x + (B[i] - x) * f).toString(16).padStart(2, '0')).join(''); };
  const shade = (a, f) => mixc(a, f < 0 ? '#000000' : '#ffffff', Math.abs(f));
  function rect(x, y, w, h, col) { c.fillStyle = col; c.fillRect(Math.round(x), Math.round(y), w, h); }
  function txt(s, x, y, col, size = 8, align = 'left') {
    c.font = size + 'px ' + PX; c.textAlign = align; c.textBaseline = 'top';
    c.fillStyle = '#000000'; c.fillText(s, Math.round(x) + 1, Math.round(y) + 1);
    c.fillStyle = col; c.fillText(s, Math.round(x), Math.round(y));
  }
  function circle(x, y, r, col) { c.fillStyle = col; c.beginPath(); c.arc(x, y, r, 0, Math.PI * 2); c.fill(); }

  function drawMartin(x, y, step, look) {
    // the GitHub avatar: a round yellow face, big round glasses, a content smile
    rect(x - 4, y - 5, 3, 5, '#2b2340'); rect(x + 1, y - 5, 3, 5, '#2b2340');
    if (step) rect(x - 5, y - 1, 4, 2, '#2b2340'); else rect(x + 1, y - 1, 4, 2, '#2b2340');
    const cy = y - 15 + (step ? -1 : 0);
    circle(x, cy, 10, '#a86a00'); circle(x, cy, 9, '#ffd23f'); circle(x - 2, cy - 3, 4, '#ffe680');
    for (const gx of [-4, 4]) { circle(x + gx, cy - 1, 4.2, '#24426e'); circle(x + gx, cy - 1, 3, '#dff3ff'); rect(x + gx - 1 + (look || 0), cy - 1, 2, 2, '#1b1b2a'); }
    rect(x - 1, cy - 2, 2, 1, '#24426e');
    c.strokeStyle = '#7a4a00'; c.lineWidth = 1.2; c.beginPath(); c.arc(x, cy + 2, 4, 0.2 * Math.PI, 0.8 * Math.PI); c.stroke();
  }
  function drawClaude(x, y, phase, tint) {
    const col = tint || '#d97757', b = phase ? -1 : 0;
    rect(x - 6, y - 4, 4, 4, shade(col, -0.35)); rect(x + 2, y - 4, 4, 4, shade(col, -0.35));
    c.fillStyle = col; c.beginPath(); c.roundRect(x - 9, y - 22 + b, 18, 18, 7); c.fill();
    rect(x - 5, y - 16 + b, 3, 3, '#1b1422'); rect(x + 2, y - 16 + b, 3, 3, '#1b1422'); rect(x - 2, y - 10 + b, 4, 1, '#1b1422');
    c.save(); c.translate(x + 9, y - 12 + b); c.rotate(phase ? -0.9 : 0.2); rect(0, -1, 9, 2, '#8a6a4a'); rect(7, -4, 4, 8, '#c9ccd6'); c.restore();
  }
  function drawCodex(x, y, phase) {
    const b = phase ? 0 : -1;
    rect(x - 6, y - 5, 3, 5, '#3a3f4f'); rect(x + 3, y - 5, 3, 5, '#3a3f4f');
    rect(x - 8, y - 12 + b, 16, 8, '#4a5165');
    rect(x - 10, y - 26 + b, 20, 15, '#262b38'); rect(x - 9, y - 25 + b, 18, 13, '#3a4256'); rect(x - 7, y - 23 + b, 14, 9, '#0e1a12');
    txt('>_', x - 6, y - 22 + b, '#5cf28f', 8);
    rect(x - 1, y - 30 + b, 2, 4, '#9aa3b8'); circle(x, y - 31 + b, 2, phase ? '#ffcf4a' : '#5cf28f');
  }
  function sprite(fn, x, y, s, ...args) { c.save(); c.translate(Math.round(x), Math.round(y)); c.scale(s, s); fn(0, 0, ...args); c.restore(); }
  function drawTineMark(x, y, s) {
    rect(x - 11 * s, y - 9 * s, 22 * s, 18 * s, '#10b981'); rect(x - 11 * s, y - 9 * s, 22 * s, 2 * s, '#6ee7b7');
    for (const k of [-5, -1, 3]) rect(x + k * s, y - 5 * s, 2 * s, 5 * s, '#ffffff');
    rect(x - 5 * s, y, 10 * s, 2 * s, '#ffffff'); rect(x - 1 * s, y + 2 * s, 2 * s, 3 * s, '#ffffff');
  }
  function drawStarIcon(x, y, col) {
    rect(x + 3, y, 2, 2, col); rect(x + 2, y + 2, 4, 2, col); rect(x, y + 3, 8, 2, col);
    rect(x + 1, y + 5, 6, 1, col); rect(x + 1, y + 6, 2, 2, col); rect(x + 5, y + 6, 2, 2, col);
  }
  function label(s, x, y, col, bg) {
    c.font = '8px ' + PX; const w = c.measureText(s).width + 8;
    rect(x - w / 2, y - 2, w, 12, bg || 'rgba(10,8,20,0.75)'); txt(s, x, y, col, 8, 'center');
  }

  // props --------------------------------------------------------------------------
  // progress of the step acting on a prop (0 before, 0..1 during, 1 after)
  function actK(sc, verb, id, t) {
    let k = 0;
    for (const st of sc.steps) if (st.act === verb + ':' + id || (id === null && st.act === verb)) k += clamp((t - st.t0) / (st.t1 - st.t0));
    return k;
  }
  function stepK(st, t, secs) { return clamp((t - st.t0) / (secs || (st.t1 - st.t0))); }
  function drawProp(p, sc, x, t, st) {
    if (p.kind === 'snail') {
      const k = actK(sc, 'overtake', p.id, t);
      const crawl = p.id === 'lag' ? -Math.max(0, t - sc.t0) * 3 : 0;
      const xx = x + crawl - 420 * ease(clamp(k * 1.3));
      if (xx < -60) return;
      const y = GROUND; c.save(); c.translate(xx, y); c.scale(1.6, 1.6); c.translate(-xx, -y);
      circle(xx, y - 16, 14, '#7c5c3b'); circle(xx, y - 16, 10, '#a77b4f'); circle(xx, y - 16, 5, '#7c5c3b');
      rect(xx - 26, y - 8, 40, 8, '#9aa86b'); rect(xx - 30, y - 14, 8, 8, '#9aa86b');
      rect(xx - 31, y - 22, 2, 8, '#9aa86b'); rect(xx - 25, y - 22, 2, 8, '#9aa86b'); rect(xx - 29, y - 12, 2, 2, '#1b1422');
      c.restore();
      if (k < 0.3) label(p.label, xx, y - 62, '#ff8a7a');
    } else if (p.kind === 'qblock') {
      const k = actK(sc, 'hit', p.id, t), st0 = sc.steps.find(s => s.act === 'hit:' + p.id);
      const hitT = st0 ? st0.t0 + 0.32 : Infinity;
      const used = t >= hitT;
      const bump = t >= hitT && t < hitT + 0.2 ? -4 * Math.sin((t - hitT) / 0.2 * Math.PI) : 0;
      const y = GROUND - 118 + bump;
      rect(x - 15, y, 30, 28, used ? '#8a6a4a' : '#e9a23b'); rect(x - 15, y, 30, 3, used ? '#a98763' : '#ffd27a'); rect(x - 15, y + 25, 30, 3, used ? '#5a4030' : '#a0601a');
      if (!used) txt('?', x, y + 6, '#fff3d6', 16, 'center');
      else {
        const f = ease(clamp((t - hitT) / 0.7));
        label(p.pop, x + 40 * f, y - 8 - 22 * f, '#fff3b0', 'rgba(60,40,10,0.85)');
      }
      void k;
    } else if (p.kind === 'bug') {
      const st0 = sc.steps.find(s => s.act === 'fight:' + p.id);
      const kf = st0 ? stepK(st0, t, 2.0) : 0;
      if (kf >= 1) return;
      const dying = clamp((kf - 0.75) / 0.25), s = (1 - dying) * 1.6;
      const y = GROUND, sh = kf > 0 ? Math.sin(t * 40) * 2 : Math.sin(t * 6);
      c.save(); c.translate(x + sh, y); c.scale(s, s);
      circle(0, -14, 13, '#3a2a2a'); circle(0, -14, 11, '#c0392b');
      for (const k of [-1, 1]) for (const j of [0, 1, 2]) rect(k * 12 + (k > 0 ? 0 : -7), -19 + j * 6, 7, 2, '#3a2a2a');
      rect(-6, -19, 4, 4, '#ffffff'); rect(2, -19, 4, 4, '#ffffff'); rect(-5, -18, 2, 2, '#000'); rect(3, -18, 2, 2, '#000');
      c.restore();
      if (dying > 0) for (let i = 0; i < 10; i++) { const a = i * 0.628, r = 34 * dying; circle(x + Math.cos(a) * r, y - 14 + Math.sin(a) * r, 2, '#ffcf4a'); }
      if (!dying) label(p.label, x, y - 62, '#ff8a7a');
      // Codex's audit beam
      if (kf > 0 && kf < 0.8 && Math.floor(t * 12) % 2 === 0) {
        c.strokeStyle = '#5cf28f'; c.lineWidth = 2; c.beginPath(); c.moveTo(SLOT.codex + 12, GROUND - 32); c.lineTo(x - 10, y - 14); c.stroke();
        c.strokeStyle = '#d97757'; c.beginPath(); c.moveTo(SLOT.claude + 16, GROUND - 22); c.lineTo(x - 10, y - 10); c.stroke();
      }
    } else if (p.kind === 'flags') {
      const st0 = sc.steps.find(s => s.act === 'raise:' + p.id);
      const k = st0 ? clamp((t - st0.t0) / 1.8) : 1;
      p.flags.forEach((name, i) => {
        const f = ease(clamp(k * 4 - i)), xx = x + i * 60, h = 34 * f;
        rect(xx, GROUND - 40, 2, 40, '#d9d4c7');
        if (f > 0) { c.fillStyle = '#e9c46a'; c.beginPath(); c.moveTo(xx + 2, GROUND - 40 + (34 - h)); c.lineTo(xx + 18, GROUND - 35 + (34 - h)); c.lineTo(xx + 2, GROUND - 30 + (34 - h)); c.fill(); }
        if (f > 0.9) txt(name, xx + 1, GROUND - 52, '#fff7e0', 8, 'center');
      });
      const f = ease(clamp(k * 4 - 3)), bx = x + 200;
      rect(bx, GROUND - 78, 3, 78, '#e8e4da');
      if (f > 0) { const y0 = GROUND - 78 + (1 - f) * 60, wv = Math.sin(t * 5) * 1.5; c.fillStyle = '#10b981'; c.beginPath(); c.moveTo(bx + 3, y0); c.lineTo(bx + 34, y0 + 8 + wv); c.lineTo(bx + 3, y0 + 16); c.fill(); }
      if (f > 0.9) txt(p.big, bx + 2, GROUND - 92, '#ffffff', 8, 'center');
    } else if (p.kind === 'machine') {
      const lvl = p.built ? 3 : actK(sc, 'build', p.id, t) * 1.5;
      const parts = [['OP LOG', '#5b6bd8'], ['DATABASE', '#8a5bd8'], ['MARKDOWN', '#3fa66b']];
      const shake = p.shaky ? Math.sin(t * 23) * (1 + actK(sc, 'wobble', null, t) * 2) : 0;
      parts.forEach(([name, col], i) => {
        const f = ease(clamp(lvl - i)); if (f <= 0) return;
        const y = GROUND - 24 - i * 26 - (1 - f) * 40;
        c.globalAlpha = f;
        rect(x - 46 + shake, y, 92, 24, col); rect(x - 46 + shake, y, 92, 2, shade(col, 0.35)); rect(x - 46 + shake, y + 22, 92, 2, shade(col, -0.4));
        txt(name, x + shake, y + 8, '#ffffff', 8, 'center');
        c.globalAlpha = 1;
      });
      if (lvl >= 2.5) {
        label('MANAGED STORAGE', x, GROUND - 104, '#e9ddff');
        for (let i = 0; i < 4; i++) { const k = (t * 0.6 + i * 0.25) % 1; circle(x + 30 + Math.sin(i * 3 + t) * 6, GROUND - 100 - k * 40, 3 + 5 * k, 'rgba(180,170,200,' + (0.4 * (1 - k)) + ')'); }
      }
      if (p.shaky) for (let i = 0; i < 5; i++) { const k = (t * 0.35 + i * 0.2) % 1; const bx = x - 40 + i * 20 - k * 90; rect(bx, GROUND - 6, 6, 4, '#c0392b'); rect(bx + 1, GROUND - 8, 1, 2, '#3a2a2a'); rect(bx + 4, GROUND - 8, 1, 2, '#3a2a2a'); }
    } else if (p.kind === 'disk') {
      const st0 = sc.steps.find(s => s.act === 'swell:' + p.id);
      const k = st0 ? clamp((t - st0.t0) / (st0.t1 - st0.t0)) : 0;
      const s = 0.55 + 0.75 * ease(k), y = GROUND, sh = Math.sin(t * 9);
      c.save(); c.translate(x, y); c.scale(s, s);
      rect(-22 + sh, -46, 44, 46, '#3b3f8f'); rect(-12 + sh, -46, 24, 14, '#c9ccd6'); rect(-15 + sh, -22, 30, 20, '#e9e4d8');
      rect(-16 + sh, -38, 5, 5, '#ff6b5b'); rect(11 + sh, -38, 5, 5, '#ff6b5b');
      c.restore();
      txt(Math.round(1 + 199 * ease(k)) + ' KB', x, y - 16 * s, '#3b3f8f', 8, 'center');
      label(p.label, x, y - 46 * s - 18, '#ff8a7a');
      // keystrokes fly from Martin into the disk
      if (k > 0 && k < 1) for (let i = 0; i < 8; i++) { const f = (t * 1.4 + i / 8) % 1; rect(MX + 14 + (x - MX - 30) * f, GROUND - 26 - Math.sin(f * Math.PI) * 30, 4, 4, '#9fe3ff'); }
      // the aha shrinks it in the player's mind: a gold "should be constant" bar
    } else if (p.kind === 'lever') {
      const st0 = sc.steps.find(s => s.act === 'pull:' + p.id);
      const f = st0 ? ease(clamp((t - st0.t0 - 0.2) / 0.5)) : 0;
      rect(x - 16, GROUND - 10, 32, 10, '#5a5a6a');
      c.save(); c.translate(x, GROUND - 10); c.rotate(-0.8 + 1.6 * f); rect(-2, -40, 4, 40, '#c9ccd6'); circle(0, -41, 7, '#ff6b5b'); c.restore();
      if (f < 0.5) label('PULL', x, GROUND - 72, '#ffcf4a');
    } else if (p.kind === 'wall') {
      if (stateAt(t).calm !== null) return;
      const st0 = sc.steps.find(s => s.act === 'bump:' + p.id);
      const k = st0 ? clamp((t - st0.t0) / 1.8) : 0, hit = k > 0 && k < 1 && Math.abs(Math.sin(k * 3 * Math.PI)) > 0.9;
      const sh = hit ? 2 : 0;
      for (let r = 0; r < 8; r++) for (let q = 0; q < 4; q++) rect(x - 24 + q * 12 + (r % 2) * 6 + sh, GROUND - 96 + r * 12, 11, 11, r % 2 ? '#6e5a7e' : '#5b4a6a');
      label(p.label, x + 6, GROUND - 112, '#ff8a7a');
    } else if (p.kind === 'pipe') {
      rect(x - 22, GROUND - 48, 44, 48, '#16a34a'); rect(x - 26, GROUND - 60, 52, 14, '#22c55e'); rect(x - 26, GROUND - 60, 52, 3, '#86efac'); rect(x - 16, GROUND - 44, 4, 44, '#4ade80');
      const enter = sc.steps.find(s => s.act === 'enter:' + p.id);
      if (enter && t < enter.t0) label('0.6.5', x, GROUND - 78, '#86efac');
    } else if (p.kind === 'box') {
      // a crate that opens on 'pop'; a buried one rises out of the ground first
      const st0 = sc.steps.find(s => s.act === 'pop:' + p.id);
      const k = st0 ? clamp((t - st0.t0) / 1.2) : 0;
      const rise = p.buried ? ease(clamp(k / 0.45)) : 1, open = p.buried ? clamp((k - 0.45) / 0.55) : clamp(k / 0.6);
      const y = GROUND + (1 - rise) * 40, sh = p.buried && k > 0 && rise < 1 ? Math.sin(t * 50) * 2 : 0;
      if (p.buried && rise < 1) { c.save(); c.beginPath(); c.rect(0, 0, W, GROUND); c.clip(); }
      rect(x - 26 + sh, y - 40, 52, 40, '#9a6b3f'); rect(x - 26 + sh, y - 40, 52, 3, '#c08a55'); rect(x - 26 + sh, y - 22, 52, 3, '#6e4a2a');
      rect(x - 3 + sh, y - 40, 6, 40, '#6e4a2a');
      if (p.buried && rise < 1) c.restore();
      if (p.buried && k > 0 && rise < 1) for (let i = 0; i < 8; i++) rect(x - 30 + rnd(i) * 60, GROUND - 4 - rnd(i + 3) * 18 * Math.sin(rise * Math.PI), 4, 4, '#6b4a2e');
      // the lid flies, the name rises
      if (open > 0) {
        c.save(); c.globalAlpha = clamp(1.5 - open * 1.5); c.translate(x + 30 * open, y - 44 - 30 * Math.sin(open * Math.PI)); c.rotate(open * 1.2); rect(-28, -4, 56, 6, '#c08a55'); c.restore();
        const f = ease(open);
        label(p.label, x, y - 56 - 26 * f, '#fff3b0', 'rgba(60,40,10,0.85)');
        for (let i = 0; i < 10; i++) { const a = i * 0.628 + t, r = 18 + 22 * f; if (f < 1) rect(x + Math.cos(a) * r, y - 46 + Math.sin(a) * r * 0.6, 3, 3, '#ffcf4a'); }
      } else {
        c.save(); c.translate(x, y - 44); rect(-28, -4, 56, 6, '#c08a55'); c.restore();
        if (!p.buried) txt('?', x, y - 34, '#fff3d6', 16, 'center');
      }
    } else if (p.kind === 'visitor') {
      // a guest model walks in at the start of its scene and works next to the party
      const f = ease(clamp((t - sc.t0) / 1.0)), xx = W + 30 + (x - W - 30) * f;
      sprite(drawClaude, xx, GROUND, SPR, f < 1 ? Math.floor(t * 7) % 2 : Math.floor(t * 5) % 2, p.col);
      label(p.name, xx, GROUND - 58, '#e9ddff');
    } else if (p.kind === 'panes') {
      // one page; 'split' slides a divider in and the second page fills
      const st0 = sc.steps.find(s => s.act === 'split:' + p.id);
      const k = st0 ? ease(clamp((t - st0.t0) / 1.0)) : 0;
      const w = 150, h = 92, x0 = x - w / 2, y0 = GROUND - h - 14;
      rect(x0, y0, w, h, '#1d2233'); rect(x0, y0, w, 10, '#323a52'); ['#ff6b5b', '#ffcf4a', '#5cf28f'].forEach((col, i) => rect(x0 + 5 + i * 8, y0 + 3, 4, 4, col));
      const split = x0 + w - (w / 2) * k;
      for (let r = 0; r < 6; r++) { rect(x0 + 8, y0 + 18 + r * 12, 4, 4, '#6ee7b7'); rect(x0 + 16, y0 + 19 + r * 12, Math.max(0, Math.min(split - x0 - 26, 40 + rnd(r) * 70)), 2, '#cfd6e6'); }
      if (k > 0) {
        rect(split - 1, y0 + 10, 2, h - 10, '#10b981');
        for (let r = 0; r < 6; r++) if (k > r / 6) { rect(split + 8, y0 + 18 + r * 12, 4, 4, '#fcd34d'); rect(split + 16, y0 + 19 + r * 12, Math.min(x0 + w - split - 26, 30 + rnd(r + 7) * 40), 2, '#cfd6e6'); }
      }
      rect(x - 4, GROUND - 14, 8, 14, '#323a52');
      label(p.label, x, y0 - 16, '#fff3b0');
    } else if (p.kind === 'lines') {
      // a signpost; 'fork' puts out one arm per line
      const st0 = sc.steps.find(s => s.act === 'fork:' + p.id);
      const k = st0 ? clamp((t - st0.t0) / 1.6) : 0;
      rect(x - 3, GROUND - 120, 6, 120, '#8a6a4a');
      const cols = ['#10b981', '#e9a23b', '#8b5cf6'];
      p.labels.forEach((name, i) => {
        const f = ease(clamp(k * 3 - i)); if (f <= 0) return;
        c.font = '8px ' + PX; const tw = c.measureText(name).width + 18, y = GROUND - 112 + i * 30;
        const ww = tw * f;
        c.fillStyle = cols[i]; c.beginPath(); c.moveTo(x, y); c.lineTo(x + ww, y); c.lineTo(x + ww + 8 * f, y + 9); c.lineTo(x + ww, y + 18); c.lineTo(x, y + 18); c.fill();
        if (f > 0.95) txt(name, x + 8, y + 5, '#ffffff', 8);
      });
      // three paths on the ground, one per line
      if (k > 0) for (let i = 0; i < 3; i++) { const f = ease(clamp(k * 3 - i)); rect(MX + 20, GROUND + 2 + i * 3, (x - MX - 20) * f, 2, cols[i]); }
    } else if (p.kind === 'bots') {
      // agent lanes working side by side; 'crash' takes them all down and brings back `keep`
      const st0 = sc.steps.find(s => s.act === 'crash:' + p.id);
      const k = st0 ? t - st0.t0 : -1;
      const down = k > 0.6 && k < 1.6, gone = k >= 0.6;
      for (let i = 0; i < p.n; i++) {
        const xx = x + i * 34;
        if (gone && i >= p.keep) { if (k < 1.2) { c.globalAlpha = clamp(1 - (k - 0.6) / 0.6); sprite(drawCodex, xx, GROUND + 30 * (k - 0.6), 1.1, 0); c.globalAlpha = 1; } continue; }
        if (down) { c.save(); c.translate(xx, GROUND); c.rotate(-1.4 * clamp((k - 0.6) / 0.2)); sprite(drawCodex, 0, 0, 1.1, 0); c.restore(); continue; }
        sprite(drawCodex, xx, GROUND + (Math.floor(t * 6 + i) % 2), 1.1, Math.floor(t * 5 + i) % 2);
      }
      label(gone && k >= 1.6 ? p.keep + ' LANES' : p.n + ' LANES', x + (p.n - 1) * 17, GROUND - 52, gone && k >= 1.6 ? '#6ee7b7' : '#cfd6e6');
      if (k > 0.35 && k < 0.85 && Math.floor(t * 14) % 2 === 0) { c.fillStyle = 'rgba(255,60,60,0.28)'; c.fillRect(0, HUD, W, GROUND - HUD); txt('OUT OF MEMORY', W / 2, 70, '#ffffff', 16, 'center'); }
    } else if (p.kind === 'sign') {
      // a claim on a board; 'defer' turns the board over to what it will say later
      const st0 = sc.steps.find(s => s.act === 'defer:' + p.id);
      const k = st0 ? clamp((t - st0.t0) / 0.9) : 0, sy = Math.cos(k * Math.PI), back = k > 0.5;
      rect(x - 3, GROUND - 70, 6, 70, '#8a6a4a');
      c.font = '8px ' + PX; const bw = Math.max(c.measureText(p.label).width, c.measureText(p.later).width) + 20;
      c.save(); c.translate(x, GROUND - 82); c.scale(1, Math.max(0.05, Math.abs(sy)));
      rect(-bw / 2, -14, bw, 28, back ? '#3a2f12' : '#1f3a2e'); rect(-bw / 2, -14, bw, 2, back ? '#ffcf4a' : '#6ee7b7');
      txt(back ? p.later : p.label, 0, -4, back ? '#ffcf4a' : '#ffffff', 8, 'center');
      c.restore();
    }
  }

  // party positions and poses for the current step --------------------------------------
  function partyPose(t, it, st8) {
    const pose = { m: { x: MX, y: 0, step: false, vis: 1 }, claude: { x: SLOT.claude, y: 0, vis: 1 }, codex: { x: SLOT.codex, y: 0, vis: 1 }, moving: false };
    const walk = Math.floor(t * 7) % 2 === 0;
    if (it.type === 'run') { pose.moving = true; pose.m.step = walk; }
    for (const k of ['claude', 'codex']) {
      const since = st8.party[k];
      if (since === undefined) { pose[k].vis = 0; continue; }
      const f = clamp((t - since) / 1.0);
      if (f < 1) { pose[k].x = W + 20 + (SLOT[k] - W - 20) * ease(f); pose[k].moving = true; }
    }
    if (it.type !== 'step' || !it.st.act) return pose;
    const st = it.st, [verb] = st.act.split(':'), k = (t - st.t0);
    if (verb === 'hit') { const f = clamp(k / 0.64); pose.m.y = -46 * Math.sin(Math.PI * f); }
    else if (verb === 'tap') { pose.m.step = Math.floor(t * 3) % 2 === 0; }
    else if (verb === 'cheer') {
      // everyone jumps for joy, slightly out of step
      ['m', 'claude', 'codex'].forEach((key, i) => { pose[key].y = -26 * Math.abs(Math.sin(k * 5 - i * 0.7)); });
    }
    else if (verb === 'scare') { if (k < 0.9) pose.m.x += Math.sin(t * 60) * 2; }
    else if (verb === 'overtake') { pose.moving = k < 1.6; pose.m.step = pose.moving && walk; }
    else if (verb === 'bump') {
      const f = clamp(k / 1.8);
      if (f < 1) pose.m.x = MX + 150 * Math.abs(Math.sin(f * 3 * Math.PI)) * (f < 0.95 ? 1 : 0);
      pose.m.step = f < 1 && walk;
    } else if (verb === 'pull') { const f = clamp(k / 0.3); pose.m.x = MX + 18 * ease(f); }
    else if (verb === 'enter') {
      // walk to the pipe, hop on, sink in (the pipe clips them in render)
      const g = clamp(k / (st.t1 - st.t0)), wf = ease(clamp(g / 0.3)), up = clamp((g - 0.05) / 0.3), sf = clamp((g - 0.5) / 0.35);
      for (const key of ['m', 'claude', 'codex']) {
        const p = pose[key], x0 = key === 'm' ? MX : SLOT[key];
        p.x = x0 + (PIPE_X - x0) * wf;
        p.y = sf > 0 ? -62 + 112 * ease(sf) : -62 * Math.sin(up * Math.PI / 2);
        if (sf >= 1) p.vis = 0;
      }
      pose.m.step = g < 0.3 && walk;
    } else if (verb === 'exit') {
      // pop out of the pipe one by one and hop back into line
      const g = k / 1.4;
      for (const key of ['m', 'claude', 'codex']) {
        const p = pose[key], x1 = key === 'm' ? MX : SLOT[key];
        const lag = key === 'm' ? 0 : key === 'claude' ? 0.15 : 0.3, f = clamp((g - lag) / 0.6);
        if (g < lag) { p.vis = 0; continue; }
        if (f < 0.4) { p.x = PIPE_X; p.y = 50 - 112 * ease(f / 0.4); }
        else { const h = (f - 0.4) / 0.6; p.x = PIPE_X + (x1 - PIPE_X) * ease(h); p.y = -62 * (1 - h) - 24 * Math.sin(Math.PI * h); }
      }
    }
    return pose;
  }

  // ── frame ─────────────────────────────────────────────────────────────────
  function skyAt(t, it, st8) {
    const wOf = sc => sc.w || S.worlds[0];
    const pal = (w, calmT) => (w.calm && calmT !== null && t >= calmT) ? w.calm : w.sky;
    if (it.type === 'run') {
      const a = wOf(it.from), b = wOf(it.to), f = clamp((t - it.t0) / (it.t1 - it.t0));
      const A = pal(a, st8.calm), B = pal(b, st8.calm);
      return { top: mixc(A[0], B[0], f), bot: mixc(A[1], B[1], f), mood: f < 0.5 ? a.mood : b.mood };
    }
    const w = wOf(it.scene);
    let P = pal(w, st8.calm);
    if (w.calm && st8.calm !== null && t - st8.calm < 1.2) { const f = clamp((t - st8.calm) / 1.2); P = [mixc(w.sky[0], w.calm[0], f), mixc(w.sky[1], w.calm[1], f)]; }
    return { top: P[0], bot: P[1], mood: st8.calm !== null && w.calm ? 'night' : w.mood };
  }

  function render(t) {
    const it = itemAt(t), st8 = stateAt(t), camx = camAt(t, it), d = dateAt(t, it);
    const sky = skyAt(t, it, st8);
    const g = c.createLinearGradient(0, HUD, 0, GROUND);
    g.addColorStop(0, sky.top); g.addColorStop(1, sky.bot);
    c.fillStyle = g; c.fillRect(0, HUD, W, H - HUD);
    if (sky.mood !== 'day') for (let k = 0; k < 50; k++) { const sx = ((rnd(k) * W - camx * 0.02) % W + W) % W, sy = HUD + rnd(k + 99) * 110; rect(sx, sy, 1, 1, 'rgba(255,255,240,' + (0.35 + 0.5 * rnd(k + 3)) + ')'); }
    if (sky.mood === 'day') { circle(560, 56, 14, 'rgba(255,240,180,0.9)'); circle(560, 56, 22, 'rgba(255,240,180,0.15)'); }
    if (sky.mood === 'storm' && (t * 0.41 % 1) < 0.02) { c.fillStyle = 'rgba(255,255,255,0.3)'; c.fillRect(0, HUD, W, GROUND - HUD); }
    for (let k = 0; k < 6; k++) {
      const cx = ((rnd(k + 40) * 900 - camx * 0.15 - t * 3) % 900 + 900) % 900 - 100, cy = HUD + 16 + rnd(k + 50) * 56;
      const col = sky.mood === 'day' ? 'rgba(255,255,255,0.7)' : 'rgba(150,140,170,0.3)';
      rect(cx, cy, 34, 6, col); rect(cx + 6, cy - 4, 20, 6, col);
    }
    for (const [sp, hgt, f] of [[0.2, 50, -0.25], [0.45, 30, -0.45]]) {
      c.fillStyle = shade(sky.bot, f); c.beginPath(); c.moveTo(0, GROUND);
      for (let x = 0; x <= W; x += 8) { const wx = x + camx * sp; c.lineTo(x, GROUND - hgt * (0.55 + 0.25 * Math.sin(wx * 0.013) + 0.2 * Math.sin(wx * 0.031 + 1))); }
      c.lineTo(W, GROUND); c.fill();
    }
    // the code tower, in the background
    // bricks are code, the scaffolding on top is tests
    const tv = towerAt(t), share = interp(testShare, d), code = tv * (1 - share), tests = tv - code;
    const th = tv / TOWER_MAX * TOWER_PX, tx = 548, tw = 46, top = GROUND - th, ctop = GROUND - code / TOWER_MAX * TOWER_PX;
    const wob = it.type === 'step' && it.st.act === 'wobble' ? Math.sin(t * 18) * 2 : 0;
    c.save(); c.beginPath(); c.rect(tx - 4, Math.max(HUD, ctop), tw + 8, GROUND - Math.max(HUD, ctop)); c.clip();
    for (let y = GROUND - 8, r = 0; y > ctop - 8; y -= 8, r++) for (let x = tx - 8; x < tx + tw; x += 16) rect(x + (r % 2) * 8 + wob * (GROUND - y) / 100, y, 15, 7, r % 3 === 0 ? '#c2410c' : '#b45309');
    c.restore();
    if (ctop - top > 2) {
      const sc0 = Math.max(HUD, top), col = 'rgba(214,222,236,0.75)';
      c.save(); c.beginPath(); c.rect(tx - 4, sc0, tw + 8, ctop - sc0); c.clip();
      rect(tx + wob, sc0, 2, ctop - sc0, col); rect(tx + tw - 2 + wob, sc0, 2, ctop - sc0, col); rect(tx + tw / 2 - 1 + wob, sc0, 2, ctop - sc0, col);
      c.strokeStyle = col; c.lineWidth = 1;
      for (let y = ctop, r = 0; y > top - 12; y -= 12, r++) { rect(tx + wob, y - 1, tw, 2, col); c.beginPath(); c.moveTo(tx + wob + (r % 2 ? tw : 0), y); c.lineTo(tx + wob + (r % 2 ? 0 : tw), y - 12); c.stroke(); }
      c.restore();
    }
    const tl = Math.max(HUD + 4, top - 24);
    if (tv >= 1000) { txt('TESTS ' + Math.round(tests / 1000) + 'k', tx + tw / 2, tl, '#d6deec', 8, 'center'); txt('CODE ' + Math.round(code / 1000) + 'k', tx + tw / 2, tl + 11, '#fde68a', 8, 'center'); }
    for (const o of overrides) if (t >= o.t && t < o.t + 1.8) {
      const before = towerAt(o.t - 1e-4); if (before <= o.v) continue;
      const k = t - o.t, btop = GROUND - before / TOWER_MAX * TOWER_PX;
      for (let i = 0; i < 30; i++) { const vx = (rnd(i) - 0.5) * 140, vy = -30 - rnd(i + 5) * 90; const px = tx + 20 + vx * k, py = btop + (GROUND - btop) * rnd(i + 9) + vy * k + 160 * k * k; if (py < GROUND) rect(px, py, 6, 4, i % 2 ? '#c2410c' : '#b45309'); }
    }
    // ground
    const grass = sky.mood === 'dark' ? '#4b5a4a' : sky.mood === 'storm' ? '#5a4a3a' : '#3fa66b';
    rect(0, GROUND, W, 4, grass); rect(0, GROUND + 4, W, H - GROUND - 4, shade(grass, -0.55));
    const bo = ((camx * 1) % 24 + 24) % 24;
    for (let r = 0; r < 2; r++) for (let x = -24; x < W + 24; x += 24) rect(x - bo + (r % 2) * 12, GROUND + 10 + r * 12, 20, 9, shade(grass, -0.45 - r * 0.08));

    // props of nearby scenes
    for (const sc of scenes) {
      if (!sc.props) continue;
      const offx = sc.cam - camx; if (offx < -W || offx > W) continue;
      for (const p of sc.props) drawProp(p, sc, p.x + offx, t, it.st);
    }

    // party
    const pose = partyPose(t, it, st8);
    const walk = Math.floor(t * 7) % 2 === 0, work = Math.floor(t * 5) % 2;
    const clip = it.type === 'step' && (it.st.act === 'enter:pipe' || it.st.act === 'exit');
    if (clip) { c.save(); c.beginPath(); c.rect(0, 0, W, GROUND - 60); c.rect(0, 0, PIPE_X - 26, H); c.rect(PIPE_X + 26, 0, W, H); c.clip(); }
    if (pose.codex.vis) sprite(drawCodex, pose.codex.x, GROUND + pose.codex.y, SPR, pose.moving || pose.codex.moving ? walk : work);
    if (pose.claude.vis) sprite(drawClaude, pose.claude.x, GROUND + pose.claude.y, SPR, pose.moving || pose.claude.moving ? walk : 1 - work);
    if (pose.m.vis) sprite(drawMartin, pose.m.x, GROUND + pose.m.y, SPR, pose.m.step, it.type === 'step' && it.st.who === 'martin' ? 1 : 0);
    if (clip) c.restore();
    // model name tags, under the agents; a ring flashes when the model changes
    for (const k of ['claude', 'codex']) {
      const e = equipAt(k, d); if (!e || !pose[k].vis || st8.party[k] === undefined) continue;
      txt(e.name, pose[k].x, GROUND + 9 + (k === 'codex' ? 12 : 0), k === 'claude' ? '#f6c6a8' : '#b6f5c9', 8, 'center');
      const since = dToSecs(e.d, t);
      if (since >= 0 && since < 1.0) { c.strokeStyle = 'rgba(255,240,160,' + (1 - since) + ')'; c.lineWidth = 2; c.beginPath(); c.arc(pose[k].x, GROUND - 24, 10 + 30 * since, 0, 7); c.stroke(); txt('NEW MODEL', pose[k].x, GROUND - 74 - 10 * since, '#fff3b0', 8, 'center'); }
    }
    // ask: a question mark over Martin; aha: LEVEL UP
    if (it.type === 'step' && it.st.mode === 'ask') { const k = t - it.st.t0; txt('?', pose.m.x + 22 + Math.sin(k * 6) * 2, GROUND - 70 + pose.m.y - Math.min(k, 1) * 8, '#c4b5fd', 16); }
    if (it.type === 'step' && it.st.act === 'scare' && t - it.st.t0 < 1.4) txt('!', pose.m.x + 20, GROUND - 70, '#ff6b5b', 16);
    for (const l of st8.lv) if (t - l.t < 2.6) {
      const k = (t - l.t) / 2.6; c.globalAlpha = clamp(1.6 - k * 1.6);
      txt('LEVEL UP!', pose.m.x + 30, GROUND - 104 - 14 * k, '#ffcf4a', 16, 'center');
      txt(l.topic, pose.m.x + 30, GROUND - 82 - 14 * k, '#fff3b0', 8, 'center');
      for (let i = 0; i < 14; i++) { const a = i * 0.449, r = 12 + 34 * k; rect(pose.m.x + Math.cos(a) * r * 1.4, GROUND - 26 + Math.sin(a) * r * 1.4, 3, 3, '#ffcf4a'); }
      c.globalAlpha = 1;
    }
    // star rain into the score
    for (const st of steps.map(s => s.st).filter(s => s.act === 'rain')) {
      const k = t - st.t0; if (k < 0 || k > 2.2) continue;
      for (let i = 0; i < 26; i++) {
        const f = clamp((k - rnd(i) * 0.9) / 1.1); if (f <= 0 || f >= 1) continue;
        const sx = 280 + rnd(i + 11) * 320, sy = HUD + 30 + rnd(i + 21) * 120;
        drawStarIcon(sx + (300 - sx) * ease(f), sy + (6 - sy) * ease(f), '#ffcf4a');
      }
    }
    // night shift: the sky goes dark, Martin sleeps, the agents keep working
    if (it.type === 'step' && it.st.act === 'night') {
      const k = t - it.st.t0, dur = it.st.t1 - it.st.t0, f = clamp(Math.min(k / 0.6, (dur - k) / 0.6));
      c.fillStyle = 'rgba(8,10,40,' + 0.6 * f + ')'; c.fillRect(0, HUD, W, GROUND - HUD + 40);
      c.globalAlpha = f; circle(560, 56, 24, mixc(sky.top, '#08102a', 0.6)); circle(560, 56, 12, '#f5f1d6'); circle(566, 52, 11, mixc(sky.top, '#08102a', 0.6));
      for (let i = 0; i < 3; i++) { const z = (k * 0.7 + i / 3) % 1; txt('z', pose.m.x + 16 + z * 24, GROUND - 48 - z * 40, '#c4d4ff', 8 + i * 4); }
      for (const key of ['claude', 'codex']) if (pose[key].vis && Math.floor(t * 8) % 2) rect(pose[key].x + 18, GROUND - 30 - rnd(Math.floor(t * 8)) * 10, 3, 3, '#ffcf4a');
      c.globalAlpha = 1;
    }
    // finale fireworks, from the moment the release flag goes up to the end
    if (finRaise && t >= finRaise.t0) {
      const k0 = finRaise.t0;
      for (let b = 0; b < 6; b++) {
        const bt = (t - k0 - b * 0.55); if (bt < 0) continue; const ph = bt % 2.4, f = ph / 1.2; if (f > 1) continue;
        const cx0 = 120 + rnd(b + Math.floor(bt / 2.4) * 7) * 400, cy0 = HUD + 30 + rnd(b + 3) * 70, col = ['#ffcf4a', '#6ee7b7', '#f472b6', '#9fe3ff'][b % 4];
        c.globalAlpha = 1 - f; for (let i = 0; i < 16; i++) { const a = i * 0.3927; rect(cx0 + Math.cos(a) * 46 * f, cy0 + Math.sin(a) * 46 * f + 14 * f * f, 4, 4, col); } c.globalAlpha = 1;
      }
    }
    // warp flash
    if (it.type === 'step' && it.st.act === 'enter:pipe') { const g2 = (t - it.st.t0) / (it.st.t1 - it.st.t0); if (g2 > 0.85) { c.fillStyle = 'rgba(255,255,255,' + clamp((g2 - 0.85) / 0.1) + ')'; c.fillRect(0, HUD, W, H - HUD); } }
    if (it.type === 'step' && it.st.act === 'exit' && t - it.st.t0 < 0.4) { c.fillStyle = 'rgba(255,255,255,' + (1 - (t - it.st.t0) / 0.4) + ')'; c.fillRect(0, HUD, W, H - HUD); }

    // world banner / skip card / end
    if (it.type === 'banner') {
      const k = t - it.t0, a = clamp(Math.min(k / 0.3, (2.3 - k) / 0.4)), w = it.scene.w;
      c.globalAlpha = a; rect(0, 78, W, 62, 'rgba(10,8,20,0.84)');
      txt('WORLD ' + w.n, W / 2, 86, '#ffcf4a', 8, 'center'); txt(w.title.toUpperCase(), W / 2, 102, '#ffffff', 16, 'center'); txt(w.sub, W / 2, 126, '#cfc8dc', 8, 'center');
      c.globalAlpha = 1;
    }
    if (it.type === 'skip') {
      const k = t - it.t0, a = clamp(Math.min(k / 0.3, (3.4 - k) / 0.4));
      c.globalAlpha = a; rect(0, HUD, W, H - HUD, 'rgba(10,8,20,0.9)');
      txt('. . .', W / 2, 96, '#ffcf4a', 16, 'center');
      wrap(it.scene.skip, 44).forEach((ln, i) => txt(ln, W / 2, 130 + i * 14, '#cfc8dc', 8, 'center'));
      c.globalAlpha = 1;
    }
    if (it.type === 'end') { c.fillStyle = 'rgba(10,8,20,' + clamp((t - it.t0) / 0.8) * 0.7 + ')'; c.fillRect(0, HUD, W, H - HUD); }

    // HUD: four things only
    rect(0, 0, W, HUD, '#0e0c16'); rect(0, HUD - 2, W, 2, '#2a2540');
    const ds = new Date(d).toUTCString().split(' ');
    txt((ds[2] + ' ' + ds[1] + ' ' + ds[3]).toUpperCase(), 8, 6, '#f3eee4', 8);
    const ver = versionAt(d); txt(ver ? ver.toUpperCase() : 'NO RELEASE YET', 152, 6, '#6ee7b7', 8);
    drawStarIcon(294, 5, '#ffcf4a'); txt(String(starsAt(d)), 306, 6, '#ffcf4a', 8);
    const lvFlash = st8.lv.some(l => t - l.t < 1.2 && Math.floor(t * 8) % 2 === 0);
    txt('LV ' + (st8.lv.length + 1), 380, 6, lvFlash ? '#ffffff' : '#c4b5fd', 8);
    const w = it.type === 'run' ? (it.to.w || it.from.w) : it.scene.w;
    if (w) txt('WORLD ' + w.n, W - 8, 6, '#8f88a6', 8, 'right');
    return { it };
  }
  function wrap(s, n) { const out = []; let ln = ''; for (const wd of s.split(' ')) { if ((ln + ' ' + wd).trim().length > n) { out.push(ln); ln = wd; } else ln = (ln + ' ' + wd).trim(); } if (ln) out.push(ln); return out; }

  // ── dialogue box ────────────────────────────────────────────────────────────
  const $ = id => document.getElementById(id);
  const face = $('face'), fc = face.getContext('2d');
  const NAME = { martin: 'MARTIN', narrator: 'STORY', system: 'PARTY', claude: 'CLAUDE', codex: 'CODEX' };
  function drawFace(who, mode) {
    fc.setTransform(1, 0, 0, 1, 0, 0); fc.clearRect(0, 0, face.width, face.height);
    const s = face.width / 48; fc.setTransform(s, 0, 0, s, 0, 0); fc.imageSmoothingEnabled = false;
    fc.fillStyle = mode === 'aha' ? '#3a2a08' : mode === 'ask' ? '#241a3d' : '#151222'; fc.fillRect(0, 0, 48, 48);
    const keep = c; c = fc;
    if (who === 'martin') { c.save(); c.translate(24, 50); c.scale(1.7, 1.7); drawMartin(0, 0, false, 1); c.restore(); }
    else if (who === 'narrator') drawTineMark(24, 24, 1.3);
    else if (who === 'system') { c.save(); c.translate(10, 10); c.scale(3.5, 3.5); drawStarIcon(0, 0, '#ffcf4a'); c.restore(); }
    c = keep;
  }
  let shownStep = null, shownFace = null;
  function updateBox(t, it) {
    let st = it.type === 'step' && it.st.text ? it.st : null, old = false;
    if (!st && it.type !== 'skip' && it.type !== 'end') { for (const s of steps) if (s.t0 <= t && s.st.text) st = s.st; old = !!st; }
    if (it.type === 'banner' || it.type === 'skip' || it.type === 'end') st = null;
    const box = $('dlg');
    box.classList.toggle('old', old);
    if (st !== shownStep) {
      shownStep = st;
      box.className = (st ? 'who-' + (st.who || 'none') + (st.mode ? ' ' + st.mode : '') : 'empty') + (old ? ' old' : '');
      const nm = !st ? '' : st.who === 'martin' && st.mode === 'ask' ? 'MARTIN ASKS' : st.who === 'martin' && st.mode === 'aha' ? 'MARTIN · LEVEL UP' : NAME[st.who] || '';
      $('dName').textContent = nm;
      $('dDate').textContent = st ? fmtDate(st.d) : '';
      $('dMore').hidden = !(st && (st.more || st.src));
    }
    const key = st ? st.who + (st.mode || '') : '';
    if (key !== shownFace) { shownFace = key; if (st) drawFace(st.who, st.mode); else { fc.setTransform(1, 0, 0, 1, 0, 0); fc.clearRect(0, 0, face.width, face.height); } }
    if (!st) { $('dShown').textContent = ''; $('dGhost').textContent = ''; return; }
    const full = st.who === 'martin' ? '“' + st.text + '”' : st.text;
    const n = old ? full.length : Math.floor((t - st.t0) * CPS);
    $('dShown').textContent = full.slice(0, n); $('dGhost').textContent = full.slice(n);
  }
  const MON = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  const fmtDate = d => { const x = new Date(d); return MON[x.getUTCMonth()] + ' ' + x.getUTCDate() + ', ' + x.getUTCFullYear(); };

  // ── sound ───────────────────────────────────────────────────────────────────
  let ac = null, drone = null;
  function audio() {
    if (ac) return ac;
    try { ac = new (window.AudioContext || window.webkitAudioContext)(); } catch (_) { return null; }
    const o = ac.createOscillator(), o2 = ac.createOscillator(), g = ac.createGain(), f = ac.createBiquadFilter();
    o.type = 'sawtooth'; o.frequency.value = 55; o2.type = 'triangle'; o2.frequency.value = 82.4; f.type = 'lowpass'; f.frequency.value = 260;
    o.connect(f); o2.connect(f); f.connect(g); g.connect(ac.destination); g.gain.value = 0; o.start(); o2.start();
    drone = g; return ac;
  }
  function tone(freq, dur, type = 'square', vol = 0.06, when = 0) {
    if (!state.sound || !audio()) return;
    const t0 = ac.currentTime + when, o = ac.createOscillator(), g = ac.createGain();
    o.type = type; o.frequency.setValueAtTime(freq, t0); g.gain.setValueAtTime(vol, t0); g.gain.exponentialRampToValueAtTime(0.0001, t0 + dur);
    o.connect(g); g.connect(ac.destination); o.start(t0); o.stop(t0 + dur + 0.02);
  }
  const seq = (notes, step = 0.08, type, vol, when = 0) => notes.forEach((n, i) => tone(n, step * 1.6, type, vol, when + i * step));
  function noise(dur, vol = 0.12) {
    if (!state.sound || !audio()) return;
    const len = ac.sampleRate * dur, buf = ac.createBuffer(1, len, ac.sampleRate), ch = buf.getChannelData(0);
    for (let i = 0; i < len; i++) ch[i] = (Math.random() * 2 - 1) * (1 - i / len);
    const s = ac.createBufferSource(), g = ac.createGain(); s.buffer = buf; g.gain.value = vol; s.connect(g); g.connect(ac.destination); s.start();
  }
  const VOICE = { martin: [620, 'square'], narrator: [392, 'triangle'], system: [988, 'square'], claude: [494, 'triangle'], codex: [784, 'square'] };
  const SFX = {
    world: () => seq([523, 659, 784, 1047], 0.09, 'square', 0.05),
    join: () => seq([440, 554, 659, 880], 0.07, 'triangle', 0.08),
    coin: (w = 0) => seq([988, 1319], 0.05, 'square', 0.04, w),
    aha: () => seq([523, 659, 784, 1047, 1319, 1568, 2093], 0.045, 'square', 0.05),
    ask: () => seq([523, 587, 659, 784], 0.05, 'triangle', 0.05),
    boss: () => seq([196, 185, 175, 165], 0.12, 'sawtooth', 0.05),
    major: () => seq([392, 523, 659, 784, 659, 784, 1047], 0.08, 'square', 0.06),
    build: () => { for (let i = 0; i < 3; i++) tone(180 + i * 40, 0.08, 'square', 0.06, i * 0.35); },
    yank: () => { noise(1.4, 0.18); seq([440, 330, 220, 110, 55], 0.12, 'sawtooth', 0.06); },
    thud: () => { for (let i = 0; i < 3; i++) tone(90, 0.12, 'square', 0.1, 0.3 + i * 0.6); },
    warp: () => seq([880, 784, 698, 587, 523, 440, 392, 330, 262], 0.07, 'square', 0.05),
    up: () => seq([262, 330, 392, 523, 659, 784], 0.06, 'square', 0.05),
    scare: () => seq([300, 200], 0.1, 'sawtooth', 0.06),
    fanfare: () => seq([523, 659, 784, 1047, 784, 1047, 1319, 1568], 0.11, 'square', 0.06),
  };
  const ACT_SFX = { hit: () => { SFX.ask(); SFX.coin(0.3); }, fight: () => { SFX.boss(); SFX.coin(1.6); }, raise: SFX.major, rain: () => { for (let i = 0; i < 6; i++) SFX.coin(i * 0.16); },
    build: SFX.build, swell: SFX.boss, pull: SFX.yank, bump: SFX.thud, enter: SFX.warp, exit: SFX.up, scare: SFX.scare, overtake: () => seq([523, 698, 880, 1047, 1397], 0.05, 'triangle', 0.07),
    night: () => seq([392, 330, 262, 196], 0.22, 'triangle', 0.05), cheer: () => { SFX.up(); SFX.coin(0.5); SFX.coin(0.8); }, pop: () => { SFX.coin(); SFX.major(); }, split: () => seq([660, 990], 0.08, 'square', 0.05),
    fork: () => { for (let i = 0; i < 3; i++) seq([523 + i * 131, 659 + i * 131], 0.06, 'square', 0.05, i * 0.53); }, crash: () => { noise(0.9, 0.2); seq([220, 165, 110], 0.15, 'sawtooth', 0.06, 0.4); },
    defer: () => seq([440, 392, 494], 0.1, 'triangle', 0.06) };
  function sounds(t0, t1) {
    if (!state.sound) return;
    for (const it of items) {
      if (!(it.t0 > t0 && it.t0 <= t1)) continue;
      if (it.type === 'banner') SFX.world();
      if (it.type === 'end') SFX.fanfare();
      if (it.type !== 'step') continue;
      const st = it.st;
      if (st.act && ACT_SFX[st.act.split(':')[0]]) ACT_SFX[st.act.split(':')[0]]();
      if (st.gain && st.gain.party) SFX.join();
      if (st.gain && st.gain.lv) SFX.aha();
    }
    { const a = dateAt(t0, itemAt(t0)), b = dateAt(t1, itemAt(t1)); for (const m of models) if (m.d > a && m.d <= b && m.d > parse('2026-06-15')) seq([330, 415, 494, 659, 831, 988], 0.04, 'sawtooth', 0.03); }
    // dialogue blips while the text types
    const it = itemAt(t1);
    if (it.type === 'step' && it.st.text && it.t0 <= t0) {
      const n0 = Math.floor((t0 - it.t0) * CPS / 3), n1 = Math.floor((t1 - it.t0) * CPS / 3), len = it.st.text.length / 3;
      if (n1 > n0 && n1 < len) { const [f, ty] = VOICE[it.st.who] || VOICE.narrator; tone(f * (1 + 0.06 * ((n1 * 7) % 3)), 0.03, ty, 0.02); }
    }
    if (drone) { const w = it.scene && it.scene.w; drone.gain.setTargetAtTime(w ? (w.drone || 0) * 0.05 : 0, ac.currentTime, 0.4); }
  }

  // ── state, loop, controls ─────────────────────────────────────────────────────
  const state = { t: 0, playing: false, speed: 1, sound: false, atStart: true, last: 0 };
  const cv = $('cv'), cx = cv.getContext('2d');
  function layout() {
    const stage = $('stage'), aw = stage.clientWidth, ah = stage.clientHeight;
    let cw = aw;
    for (let i = 0; i < 3; i++) { const fs = clamp(cw * 0.026, 15, 21); const dh = Math.round(fs * 1.4 * 3 + fs + 30); cw = Math.min(aw, (ah - dh) * W / H); }
    cw = Math.floor(Math.max(cw, 280)); const ch = Math.floor(cw * H / W), dpr = window.devicePixelRatio || 1;
    const fs = clamp(cw * 0.026, 15, 21);
    cv.style.width = cw + 'px'; cv.style.height = ch + 'px'; cv.width = Math.round(cw * dpr); cv.height = Math.round(ch * dpr);
    const box = $('screen'); box.style.width = cw + 'px';
    $('dlg').style.fontSize = fs + 'px';
    face.width = face.height = Math.round(fs * 3.4 * dpr / 48) * 48 || 96; shownFace = null;
    draw();
  }
  function draw() {
    const { it } = render(state.t);
    cx.imageSmoothingEnabled = false; cx.drawImage(off, 0, 0, cv.width, cv.height);
    updateBox(state.t, it);
    if (!state.dragging) $('scrub').value = Math.round(state.t * 100);
    const w = it.scene && it.scene.w;
    $('worldName').textContent = it.type === 'end' ? 'Tine ' + FIN.tag.replace(/^v/, '') + ' · ' + FIN.name : w ? 'World ' + w.n + ' · ' + w.title : '';
    $('finale').hidden = !(it.type === 'end' && state.t - it.t0 > 1.6);
    $('start').hidden = !state.atStart;
  }
  function frame(now) {
    const dt = Math.min(0.05, (now - (state.last || now)) / 1000); state.last = now;
    if (state.playing) { const t0 = state.t; state.t = Math.min(T_END, state.t + dt * state.speed); sounds(t0, state.t); if (state.t >= T_END) setPlaying(false); }
    draw(); requestAnimationFrame(frame);
  }
  function setPlaying(p) {
    state.playing = p;
    if (p) { state.atStart = false; if (state.t >= T_END) state.t = 0; }
    $('play').textContent = p ? 'Pause' : 'Play'; $('play').setAttribute('aria-pressed', String(p));
    if (!p && drone) drone.gain.setTargetAtTime(0, ac.currentTime, 0.2);
  }
  function seek(t) { state.t = clamp(t, 0, T_END); state.atStart = false; draw(); }
  // advance: finish typing first, then jump to the next line
  function advance() {
    const it = itemAt(state.t);
    if (it.type === 'step' && it.st.text) { const done = it.t0 + it.st.text.length / CPS + 0.05; if (state.t < done) return seek(done); }
    const nx = items.find(x => x.t0 > state.t + 0.01 && (x.type === 'step' || x.type === 'banner' || x.type === 'end'));
    if (nx) seek(nx.t0 + 0.001);
  }
  function back() {
    const cur = itemAt(state.t);
    const pv = [...items].reverse().find(x => x.t0 < cur.t0 - 0.01 && (x.type === 'step' || x.type === 'banner'));
    seek(pv ? pv.t0 + 0.001 : 0);
  }
  function buildScrub() {
    const parts = [];
    for (const w of S.worlds) parts.push(`${w.sky[1]} ${(w.t0 / T_END * 100).toFixed(2)}%`, `${w.sky[1]} ${(w.t1 / T_END * 100).toFixed(2)}%`);
    for (const it of items) if (it.type === 'skip') parts.push(`#3a3450 ${(it.t0 / T_END * 100).toFixed(2)}%`, `#3a3450 ${(it.t1 / T_END * 100).toFixed(2)}%`);
    parts.sort((a, b) => parseFloat(a.split(' ')[1]) - parseFloat(b.split(' ')[1]));
    $('band').style.background = `linear-gradient(90deg, ${parts.join(', ')})`;
    const ticks = $('ticks'); ticks.innerHTML = '';
    for (const w of S.worlds) {
      const b = document.createElement('button'); b.className = 'tick'; b.style.left = (w.t0 / T_END * 100) + '%'; b.textContent = w.n;
      b.title = 'World ' + w.n + ': ' + w.title; b.setAttribute('aria-label', b.title); b.addEventListener('click', () => seek(w.t0 + 0.01)); ticks.appendChild(b);
    }
    const fs = scenes.find(s => s.final);
    if (fs) {
      const f = document.createElement('button'); f.className = 'tick fin'; f.style.left = (fs.t0 / T_END * 100) + '%'; f.textContent = FIN.tag.replace(/^v/, '');
      f.title = 'Tine ' + FIN.tag + ': ' + FIN.name; f.setAttribute('aria-label', f.title); f.addEventListener('click', () => seek(fs.t0 + 0.01)); ticks.appendChild(f);
    }
    $('scrub').max = Math.round(T_END * 100);
  }
  // the finale panel: what 0.8.0 brings, what Martin learned, the tally, the people
  function buildFinale() {
    $('finTag').textContent = 'TINE ' + FIN.tag.replace(/^v/, '') + (FIN.pending ? ' (release pending)' : ' · STABLE · ' + fmtDate(parse(FIN.at)).toUpperCase());
    $('finName').textContent = FIN.name;
    const tr = $('trophies'); tr.innerHTML = '';
    FIN.trophies.forEach((x, i) => { const li = document.createElement('li'); li.style.setProperty('--i', i); const b = document.createElement('b'); b.textContent = x.label; const s = document.createElement('span'); s.textContent = x.text; li.append(b, s); tr.appendChild(li); });
    const sy = $('syllabus'); sy.innerHTML = '';
    for (const ch of FIN.syllabus || []) { const li = document.createElement('li'); const b = document.createElement('b'); b.textContent = ch.chapter; const q = document.createElement('span'); q.textContent = '“' + ch.q + '”'; li.append(b, q); sy.appendChild(li); }
    const total = D.commits.reduce((a, r) => a + r[1], 0);
    const peak = D.loc.reduce((a, r) => Math.max(a, r[2]), 0);
    const inst = ['Windows', 'Linux', 'Android', 'macOS'].reduce((a, k) => a + (D.downloads[k] || 0), 0);
    const tally = [
      ['★', D.stars[D.stars.length - 1][1], 'GitHub stars'],
      ['', total.toLocaleString('en-US'), 'commits'],
      ['', D.releases.length + (finInData ? 0 : 1), 'releases, Betas included'],
      ['', inst.toLocaleString('en-US'), 'installer downloads'],
      ['', Math.round(peak / 1000) + 'k', 'peak lines of Rust, before the cut'],
      ['?', (D.learning || []).reduce((a, r) => a + r[2], 0), 'times Martin asked the agents to explain something'],
      ['', D.credits.length, 'people who reported, asked, argued'],
    ];
    $('tally').innerHTML = '';
    for (const [p, v, l] of tally) { const li = document.createElement('li'); const b = document.createElement('b'); b.textContent = (p ? p + ' ' : '') + v; const s = document.createElement('span'); s.textContent = l; li.append(b, s); $('tally').appendChild(li); }
    $('closing').textContent = FIN.closing;
    const cr = $('creditsList'); cr.innerHTML = '';
    for (const [login] of D.credits) { const li = document.createElement('li'); li.textContent = login; cr.appendChild(li); }
  }
  function openCard() {
    const st = shownStep; if (!st) return; setPlaying(false);
    $('cardDate').textContent = fmtDate(st.d);
    $('cardQuote').textContent = st.who === 'martin' ? '“' + st.text + '”' : st.text;
    $('cardMore').hidden = !st.more; $('cardMore').textContent = st.more || '';
    $('cardSrc').hidden = !st.src; $('cardSrc').textContent = st.src ? 'Source: ' + st.src : '';
    $('card').hidden = false; $('cardClose').focus();
  }
  function bind() {
    $('play').addEventListener('click', () => setPlaying(!state.playing));
    $('startPlay').addEventListener('click', () => setPlaying(true));
    $('restart').addEventListener('click', () => { seek(0); setPlaying(true); });
    $('speed').addEventListener('click', () => { state.speed = state.speed === 1 ? 1.5 : state.speed === 1.5 ? 0.75 : 1; $('speed').textContent = state.speed + '×'; });
    $('sound').addEventListener('click', () => {
      state.sound = !state.sound; $('sound').textContent = state.sound ? 'Sound on' : 'Sound off'; $('sound').setAttribute('aria-pressed', String(state.sound));
      if (state.sound) { audio(); if (ac && ac.state === 'suspended') ac.resume(); SFX.coin(); } else if (drone) drone.gain.setTargetAtTime(0, ac.currentTime, 0.1);
    });
    $('fs').addEventListener('click', () => { const el = document.documentElement; try { if (document.fullscreenElement) document.exitFullscreen(); else if (el.requestFullscreen) el.requestFullscreen().catch(() => {}); } catch (_) {} });
    $('dMore').addEventListener('click', ev => { ev.stopPropagation(); openCard(); });
    $('dlg').addEventListener('click', advance);
    $('cardClose').addEventListener('click', () => { $('card').hidden = true; });
    $('finReplay').addEventListener('click', () => { seek(0); setPlaying(true); });
    const sc = $('scrub');
    sc.addEventListener('input', () => { state.dragging = true; state.t = sc.value / 100; state.atStart = false; draw(); });
    sc.addEventListener('change', () => { state.dragging = false; });
    cv.addEventListener('click', () => setPlaying(!state.playing));
    window.addEventListener('keydown', ev => {
      if (ev.key === 'Escape') { $('card').hidden = true; return; }
      if (!$('card').hidden) return;
      if (ev.key === ' ' && ev.target.tagName !== 'BUTTON') { ev.preventDefault(); setPlaying(!state.playing); }
      else if ((ev.key === 'ArrowRight' || ev.key === 'Enter') && ev.target.id !== 'scrub') { ev.preventDefault(); advance(); }
      else if (ev.key === 'ArrowLeft' && ev.target.id !== 'scrub') { ev.preventDefault(); back(); }
    });
    window.addEventListener('resize', layout);
  }
  function start() {
    buildScrub(); buildFinale(); bind();
    // to the nearest half minute: "4½ minutes"
    const halves = Math.round(T_END / 30);
    $('runtime').textContent = Math.floor(halves / 2) + (halves % 2 ? '½' : '') + ' minutes';
    layout(); requestAnimationFrame(frame);
  }
  const ready = document.fonts && document.fonts.load ? document.fonts.load('8px "Press Start 2P"').catch(() => {}) : Promise.resolve();
  Promise.race([ready, new Promise(r => setTimeout(r, 1500))]).then(start);
  window.TINE_ENGINE = { T_END, items, seek: t => seek(t) };
})();
