// A virtual display arrangement controls cross-computer boundaries. Actual OS
// rectangles remain the stable monitor identities and input coordinate spaces.
(() => {
  const canvas = document.querySelector('.layout-canvas');
  const opposite = { left: 'right', right: 'left', top: 'bottom', bottom: 'top' };
  const geometry = { windows: [], mac: [] }, tiles = new Map();
  const key = JSON.stringify, clone = (v) => v == null ? null : JSON.parse(key(v));
  const rect = (d) => [Number(d.originX), Number(d.originY), Number(d.width), Number(d.height)];
  const same = (a, b) => key(a) === key(b);
  const space = document.createElement('div'); space.className = 'monitor-space'; canvas.append(space);
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.classList.add('display-connections'); svg.setAttribute('aria-hidden', 'true'); canvas.append(svg);
  canvas.classList.add('per-display', 'arranged-displays');
  let choice = null, drag = null, queued = false, camera = { scale: 1, x: 0, y: 0 }, invalid = false;
  const edge = () => window.EdgeMouseLayout?.getEdge() ?? 'right';
  const ready = () => geometry.windows.length && geometry.mac.length && canvas.querySelector('.mini-screen').getAttribute('aria-disabled') !== 'true';
  const overlap = (a, b) => Math.max(a[0], b[0]) < Math.min(a[0] + a[2], b[0] + b[2]) - 1e-6 && Math.max(a[1], b[1]) < Math.min(a[1] + a[3], b[1] + b[3]) - 1e-6;

  function automatic(displays, facing, centered = false) {
    const vertical = ['left', 'right'].includes(facing);
    const scale = 1000 / Math.max(1, displays.reduce((sum, d) => sum + d[vertical ? 3 : 2], 0));
    const order = displays.map((r, i) => ({ r, i })).sort((a, b) => vertical ? a.r[1] - b.r[1] || a.r[0] - b.r[0] : a.r[0] - b.r[0] || a.r[1] - b.r[1]);
    const crossSize = Math.max(0, ...displays.map((r) => r[vertical ? 2 : 3] * scale));
    const frames = []; let offset = 0;
    for (const { r, i } of order) {
      const w = r[2] * scale, h = r[3] * scale;
      frames[i] = { left: [0, offset, w, h], right: [-w, offset, w, h], top: [offset, 0, w, h], bottom: [offset, -h, w, h] }[facing];
      if (centered) frames[i][vertical ? 0 : 1] += (crossSize - (vertical ? w : h)) / 2 * (['right', 'bottom'].includes(facing) ? -1 : 1);
      offset += vertical ? h : w;
    }
    return frames;
  }
  function defaultChoice(direction = edge(), centered = false) {
    const windows = geometry.windows.map(rect), mac = geometry.mac.map(rect);
    return { macOn: direction, windows, mac, positions: { windows: automatic(windows, direction, centered), mac: automatic(mac, opposite[direction], centered) } };
  }
  function entries() {
    // A connected implicit layout must still match existing service defaults.
    // Local-only previews and newly saved arrangements can use centered frames.
    const selected = drag?.draft ?? choice ?? defaultChoice(edge(), !geometry.windows.length || !geometry.mac.length);
    const fallback = defaultChoice(selected.macOn);
    return ['windows', 'mac'].flatMap((side) => geometry[side].map((display, index) => {
      const actual = rect(display), at = selected[side].findIndex((r) => same(r, actual));
      const frames = selected.positions?.[side] ?? automatic(selected[side], side === 'windows' ? selected.macOn : opposite[selected.macOn]);
      return { id: `${side}:${key(actual)}`, side, actual, display, index, frame: at >= 0 ? [...frames[at]] : [...fallback.positions[side][index]], missing: at < 0 };
    }));
  }
  function currentChoice(items = entries()) {
    const center = (side, axis) => { const rows = items.filter((e) => e.side === side); return rows.reduce((sum, e) => sum + e.frame[axis] + e.frame[axis + 2] / 2, 0) / Math.max(1, rows.length); };
    const dx = center('mac', 0) - center('windows', 0), dy = center('mac', 1) - center('windows', 1);
    return { macOn: Math.abs(dx) >= Math.abs(dy) ? dx < 0 ? 'left' : 'right' : dy < 0 ? 'top' : 'bottom',
      windows: items.filter((e) => e.side === 'windows').map((e) => e.actual), mac: items.filter((e) => e.side === 'mac').map((e) => e.actual),
      positions: { windows: items.filter((e) => e.side === 'windows').map((e) => e.frame), mac: items.filter((e) => e.side === 'mac').map((e) => e.frame) } };
  }
  function exposed(item, facing) {
    const r = item.actual, vertical = ['left', 'right'].includes(facing), axis = vertical ? 1 : 0, size = axis + 2;
    let ranges = [[r[axis], r[axis] + r[size]]];
    for (const d of geometry[item.side]) {
      const o = rect(d);
      const covered = { left: o[0] < r[0] && o[0] + o[2] >= r[0], right: o[0] + o[2] > r[0] + r[2] && o[0] <= r[0] + r[2], top: o[1] < r[1] && o[1] + o[3] >= r[1], bottom: o[1] + o[3] > r[1] + r[3] && o[1] <= r[1] + r[3] }[facing];
      if (!covered) continue;
      const a = o[axis], b = a + o[size];
      ranges = ranges.flatMap(([start, end]) => a >= end || b <= start ? [[start, end]] : [[start, Math.min(a, end)], [Math.max(b, start), end]].filter(([x, y]) => y > x));
    }
    return ranges.map(([start, end]) => [item.frame[axis] + (start - r[axis]) / r[size] * item.frame[size], item.frame[axis] + (end - r[axis]) / r[size] * item.frame[size]]);
  }
  function routes(items = entries()) {
    const result = [];
    for (const a of items.filter((e) => e.side === 'windows' && !e.missing)) {
      for (const b of items.filter((e) => e.side === 'mac' && !e.missing)) {
        for (const facing of ['left', 'right', 'top', 'bottom']) {
          const x = a.frame, y = b.frame;
          const distance = { left: x[0] - y[0] - y[2], right: x[0] + x[2] - y[0], top: x[1] - y[1] - y[3], bottom: x[1] + x[3] - y[1] }[facing];
          if (Math.abs(distance) > 1e-6) continue;
          for (const [a0, a1] of exposed(a, facing)) for (const [b0, b1] of exposed(b, opposite[facing])) {
            const from = Math.max(a0, b0), to = Math.min(a1, b1);
            if (to - from > 1e-6) result.push({ a, b, facing, from, to });
          }
        }
      }
    }
    return result;
  }
  function snap(frame, others) {
    const result = [...frame], threshold = 12 / camera.scale;
    for (const axis of [0, 1]) {
      let best = threshold;
      for (const { frame: o } of others) {
        for (const coordinate of [o[axis] - frame[axis + 2], o[axis] + o[axis + 2], o[axis], o[axis] + o[axis + 2] - frame[axis + 2], o[axis] + (o[axis + 2] - frame[axis + 2]) / 2]) {
          const distance = Math.abs(frame[axis] - coordinate);
          if (distance < best) { best = distance; result[axis] = coordinate; }
        }
      }
    }
    return result;
  }
  function commit(items) {
    choice = currentChoice(items);
    window.EdgeMouseLayout.setEdge(choice.macOn);
    window.EdgeMouseLayout.changed();
  }
  function begin(event, id) {
    if (event.button !== 0 || !ready() || drag) return;
    const items = entries(), item = items.find((e) => e.id === id);
    event.preventDefault(); event.currentTarget.focus({ preventScroll: true });
    drag = { id, pointerId: event.pointerId, x: event.clientX, y: event.clientY, original: [...item.frame], draft: currentChoice(items), moved: false };
    event.currentTarget.setPointerCapture(event.pointerId);
    canvas.classList.add('is-dragging');
  }
  window.addEventListener('pointermove', (event) => {
    if (!drag || event.pointerId !== drag.pointerId) return;
    const dx = (event.clientX - drag.x) / camera.scale, dy = (event.clientY - drag.y) / camera.scale;
    drag.moved ||= Math.hypot(event.clientX - drag.x, event.clientY - drag.y) > 5;
    if (!drag.moved) return;
    const items = entries(), item = items.find((e) => e.id === drag.id);
    if (!item) { finish({ type: 'pointercancel', pointerId: drag.pointerId }); return; }
    item.frame = snap([drag.original[0] + dx, drag.original[1] + dy, drag.original[2], drag.original[3]], items.filter((e) => e !== item));
    invalid = items.some((other) => other !== item && overlap(item.frame, other.frame));
    drag.draft = currentChoice(items); render();
  });
  function finish(event) {
    if (!drag || event.pointerId !== drag.pointerId) return;
    const items = entries();
    const save = drag.moved && !invalid && event.type !== 'pointercancel' && !items.some((e, i) => items.slice(0, i).some((o) => overlap(e.frame, o.frame)));
    const captured = tiles.get(drag.id), pointerId = drag.pointerId;
    drag = null;
    if (captured?.hasPointerCapture(pointerId)) captured.releasePointerCapture(pointerId); invalid = false; canvas.classList.remove('is-dragging');
    if (save) commit(items);
    render();
  }
  window.addEventListener('pointerup', finish, true);
  window.addEventListener('pointercancel', finish, true);
  window.addEventListener('blur', () => { if (drag) finish({ type: 'pointercancel', pointerId: drag.pointerId }); });
  window.addEventListener('keydown', (event) => { if (event.key === 'Escape' && drag) finish({ type: 'pointercancel', pointerId: drag.pointerId }); });
  function keyboard(event, id) {
    const movement = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[event.key];
    if (!movement || !ready() || drag) return;
    event.preventDefault();
    const items = entries(), item = items.find((e) => e.id === id), step = event.shiftKey ? 100 : 10;
    // A small keyboard step deliberately moves away from an existing edge;
    // dragging supplies magnetic snapping when arranging a new contact.
    item.frame[0] += movement[0] * step; item.frame[1] += movement[1] * step;
    if (!items.some((other) => other !== item && overlap(item.frame, other.frame))) { commit(items); render(); }
  }
  function render() {
    const items = entries();
    if (!drag && items.length && canvas.clientWidth) {
      const left = Math.min(...items.map((e) => e.frame[0])), top = Math.min(...items.map((e) => e.frame[1]));
      const width = Math.max(...items.map((e) => e.frame[0] + e.frame[2])) - left;
      const height = Math.max(...items.map((e) => e.frame[1] + e.frame[3])) - top;
      const scale = Math.max(0.01, Math.min((canvas.clientWidth - 84) / Math.max(1, width), (canvas.clientHeight - 150) / Math.max(1, height)));
      camera = { scale, x: (canvas.clientWidth - width * scale) / 2 - left * scale, y: 48 + (canvas.clientHeight - 150 - height * scale) / 2 - top * scale };
    }
    for (const [id, tile] of tiles) if (!items.some((e) => e.id === id)) { tile.remove(); tiles.delete(id); }
    for (const item of items) {
      let tile = tiles.get(item.id);
      if (!tile) {
        tile = document.createElement('button'); tile.type = 'button'; tile.className = `display-tile ${item.side === 'windows' ? 'windows' : 'mac'}-wallpaper`;
        tile.dataset.displaySide = item.side; tile.dataset.rect = key(item.actual);
        const title = document.createElement('b'); title.textContent = `${item.side === 'windows' ? 'Windows' : 'Mac'} · ${item.display.primary ? '主屏' : item.index + 1}`;
        const resolution = document.createElement('small'); resolution.textContent = `${item.display.pixelWidth || item.actual[2]} × ${item.display.pixelHeight || item.actual[3]}`;
        tile.append(title, resolution); tile.setAttribute('aria-label', `${title.textContent}，方向键移动屏幕，Shift 加速`);
        tile.addEventListener('pointerdown', (e) => begin(e, item.id)); tile.addEventListener('lostpointercapture', (e) => finish({ type: 'pointercancel', pointerId: e.pointerId })); tile.addEventListener('keydown', (e) => keyboard(e, item.id));
        tiles.set(item.id, tile); space.append(tile);
      }
      tile.disabled = !ready(); tile.classList.toggle('is-moving', drag?.id === item.id); tile.classList.toggle('is-overlapping', invalid && drag?.id === item.id);
      const [x, y, w, h] = item.frame;
      Object.assign(tile.style, { left: `${camera.x + x * camera.scale}px`, top: `${camera.y + y * camera.scale}px`, width: `${w * camera.scale}px`, height: `${h * camera.scale}px` });
    }
    svg.replaceChildren(); svg.setAttribute('viewBox', `0 0 ${Math.max(1, canvas.clientWidth)} ${Math.max(1, canvas.clientHeight)}`);
    const links = invalid ? [] : routes(items);
    links.forEach(({ a, facing, from, to }, index) => {
      const vertical = ['left', 'right'].includes(facing), f = a.frame;
      const boundary = { left: f[0], right: f[0] + f[2], top: f[1], bottom: f[1] + f[3] }[facing];
      const p = vertical ? [boundary, from] : [from, boundary], q = vertical ? [boundary, to] : [to, boundary];
      const line = document.createElementNS(svg.namespaceURI, 'line');
      for (const [name, value] of Object.entries({ x1: camera.x + p[0] * camera.scale, y1: camera.y + p[1] * camera.scale, x2: camera.x + q[0] * camera.scale, y2: camera.y + q[1] * camera.scale, stroke: index % 2 ? '#548edc' : '#0bafa3', 'stroke-width': 7, 'stroke-linecap': 'round' })) line.setAttribute(name, value);
      svg.append(line);
    });
    const changed = choice && ['windows', 'mac'].some((side) => choice[side].length !== geometry[side].length || choice[side].some((r) => !geometry[side].some((d) => same(rect(d), r))));
    document.querySelector('.display-selection-status').textContent = !ready() ? '连接两台电脑后可拖动屏幕' : invalid ? '屏幕不能重叠，请移到相邻边缘' : changed ? '屏幕已变化，请重新排列并确认穿越边缘' : links.length ? '高亮边缘可以穿越，位置重叠的区域自动对应' : '屏幕之间有空隙，将边缘贴合即可穿越';
    document.querySelector('[data-display-all]').disabled = !ready();
  }
  function schedule() { if (!queued) { queued = true; requestAnimationFrame(() => { queued = false; render(); }); } }
  window.EdgeMouseDisplays = {
    get: () => clone(choice), apply(value) { choice = clone(value); render(); },
    setEdge(value, arrange = false) { if (arrange && ready()) choice = defaultChoice(value, true); else if (choice) choice.macOn = value; render(); },
    update(side, displays) { geometry[side] = displays.filter((d, i) => !displays.slice(0, i).some((other) => same(rect(other), rect(d)))); schedule(); },
    refresh: schedule, routes,
  };
  document.querySelector('[data-display-all]').addEventListener('click', () => { if (ready()) { choice = defaultChoice(edge(), true); window.EdgeMouseLayout.changed(); render(); } });
  if (!window.__TAURI__) {
    geometry.windows = [{ originX: 0, originY: 0, width: 2160, height: 3840, pixelWidth: 2160, pixelHeight: 3840, primary: true }];
    geometry.mac = [{ originX: 0, originY: -1080, width: 1920, height: 1080, pixelWidth: 1920, pixelHeight: 1080, primary: false }, { originX: 240, originY: 0, width: 1470, height: 956, pixelWidth: 2940, pixelHeight: 1912, primary: true }];
  }
  new ResizeObserver(schedule).observe(canvas);
})();
