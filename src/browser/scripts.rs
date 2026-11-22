//! The scripts BenCode runs in a browser page. `PRELUDE` installs
//! `window.__bencode` (console capture, network capture, dialog hooks,
//! address changes, selectors, Shadow DOM query); every other script starts
//! with it, since a page loaded before it (an error page, `about:blank`) has
//! none. Each script evaluates to a JSON string.

/// Console capture, network (`fetch`/`XHR`) capture, history hooks, dialog
/// hooks and helpers the other scripts use. Idempotent.
pub const PRELUDE: &str = r#"(() => {
  if (window.__bencode) return;
  const post = (m) => { try { window.ipc.postMessage(JSON.stringify(m)); } catch (_) {} };
  const logs = [];
  // A logged object is cut short as it is read: a page may log its whole
  // state on every render, and nobody may ever ask for these lines.
  const brief = (v, depth) => {
    if (typeof v === 'function') return '[Function]';
    if (typeof v === 'bigint' || typeof v === 'symbol') return String(v);
    if (v === null || typeof v !== 'object') return v;
    if (v instanceof Date) return String(v);
    if (v instanceof Element) return `<${v.localName}>`;
    if (depth >= 3) return Array.isArray(v) ? '[…]' : '{…}';
    if (Array.isArray(v)) {
      const out = v.slice(0, 30).map((x) => brief(x, depth + 1));
      if (v.length > 30) out.push('…');
      return out;
    }
    const out = {};
    let n = 0;
    for (const k in v) {
      if (n++ >= 30) { out['…'] = ''; break; }
      try { out[k] = brief(v[k], depth + 1); } catch (_) { out[k] = '[unreadable]'; }
    }
    return out;
  };
  const fmt = (a) => {
    if (typeof a === 'string') return a;
    if (a instanceof Error) return a.stack || String(a);
    try {
      const text = JSON.stringify(brief(a, 0));
      return text === undefined ? String(a) : text;
    } catch (_) { return String(a); }
  };
  const keep = (level, text) => {
    logs.push({ level, text: String(text).slice(0, 2000), t: Date.now() });
    if (logs.length > 300) logs.shift();
  };
  for (const level of ['log', 'info', 'warn', 'error', 'debug']) {
    const original = console[level];
    console[level] = function (...args) {
      keep(level, args.map(fmt).join(' '));
      return original.apply(this, args);
    };
  }
  window.addEventListener('error', (e) =>
    keep('error', e.message + (e.filename ? ` (${e.filename}:${e.lineno})` : '')));
  // A script, stylesheet or image that fails to load says so on its
  // element, which the listener above never hears: a module that fails
  // (or one it imports) leaves the page blank with nothing else logged.
  window.addEventListener('error', (e) => {
    const el = e.target;
    if (!(el instanceof Element)) return;
    const url = el.currentSrc || el.src || el.href;
    const what = el.type === 'module' ? 'module script (or one of its imports)' : el.localName;
    keep('error', `Failed to load ${what}` + (typeof url === 'string' && url ? `: ${url}` : ''));
  }, true);
  window.addEventListener('unhandledrejection', (e) =>
    keep('error', 'Unhandled rejection: ' + fmt(e.reason)));
  // Only that it moved: the app asks the view where, not the page.
  const moved = () => post({ type: 'moved' });
  for (const name of ['pushState', 'replaceState']) {
    const original = history[name];
    history[name] = function (...args) { const r = original.apply(this, args); moved(); return r; };
  }
  window.addEventListener('popstate', moved);
  window.addEventListener('hashchange', moved);

  // Network capture (fetch + XMLHttpRequest): bounded ring buffer, never
  // buffers binary bodies and redacts password-like keys in text previews.
  const net = [];
  let nextNetId = 1;
  const secretKey = (k) => /pass|secret|token|api_?key/i.test(k);
  // `a=1&password=…`: a form's fields, masked by name like FormData's.
  const redactForm = (params) => {
    const parts = [];
    for (const [k, v] of params.entries()) {
      parts.push(`${encodeURIComponent(k)}=${secretKey(k) ? '(redacted)' : encodeURIComponent(v)}`);
    }
    return parts.join('&');
  };
  const redactBody = (raw) => {
    if (raw == null) return null;
    let s;
    if (typeof raw === 'string') {
      s = /^[^\s{}\[\]=&]+=/.test(raw) ? redactForm(new URLSearchParams(raw)) : raw;
    } else if (raw instanceof URLSearchParams) s = redactForm(raw);
    else if (raw instanceof FormData) {
      const parts = [];
      for (const [k, v] of raw.entries()) {
        parts.push(`${k}=${secretKey(k) ? '(redacted)' : (typeof v === 'string' ? v.slice(0, 200) : '[File]')}`);
      }
      s = parts.join('&');
    } else return `[${Object.prototype.toString.call(raw)}]`;
    s = s.replace(/("(?:password|passwd|secret|access_token|refresh_token|apiKey|api_key)"\s*:\s*)"[^"]*"/gi, '$1"(redacted)"');
    return s.length > 4096 ? s.slice(0, 4096) + '…' : s;
  };
  const isTextType = (ct) => /json|text|xml|javascript|svg|form-urlencoded/i.test(ct || '');
  const pushNet = (entry) => {
    net.push(entry);
    if (net.length > 200) net.shift();
    return entry;
  };
  const state = {
    logs,
    net,
    inFlight: 0,
    post,
    refs: new Map(),
    picker: null,
    dialogs: [],
    dialogArmed: null,
  };

  if (typeof window.fetch === 'function') {
    const origFetch = window.fetch;
    window.fetch = function (input, init) {
      const url = typeof input === 'string' ? input : (input && input.url ? input.url : String(input));
      const method = ((init && init.method) || (input && input.method) || 'GET').toUpperCase();
      const t0 = Date.now();
      const entry = pushNet({
        id: nextNetId++,
        kind: 'fetch',
        method,
        url,
        status: null,
        ok: null,
        durationMs: null,
        error: null,
        reqBody: init && init.body !== undefined ? redactBody(init.body) : null,
        resType: null,
        resBody: null,
        t: t0,
      });
      state.inFlight += 1;
      const done = () => { state.inFlight = Math.max(0, state.inFlight - 1); };
      return origFetch.apply(this, arguments).then((res) => {
        entry.status = res.status;
        entry.ok = res.ok;
        entry.durationMs = Date.now() - t0;
        const ct = (res.headers && res.headers.get('content-type')) || '';
        entry.resType = ct.split(';')[0].trim() || null;
        if (isTextType(ct)) {
          try {
            res.clone().text().then((txt) => { entry.resBody = redactBody(txt); }).catch(() => {});
          } catch (_) {}
        }
        done();
        return res;
      }, (err) => {
        entry.durationMs = Date.now() - t0;
        entry.error = err && err.message ? err.message : String(err);
        done();
        throw err;
      });
    };
  }

  if (typeof XMLHttpRequest !== 'undefined') {
    const origOpen = XMLHttpRequest.prototype.open;
    const origSend = XMLHttpRequest.prototype.send;
    XMLHttpRequest.prototype.open = function (method, url) {
      this.__bc = { method: String(method || 'GET').toUpperCase(), url: String(url || '') };
      return origOpen.apply(this, arguments);
    };
    XMLHttpRequest.prototype.send = function (body) {
      const info = this.__bc || { method: 'GET', url: '' };
      const t0 = Date.now();
      const entry = pushNet({
        id: nextNetId++,
        kind: 'xhr',
        method: info.method,
        url: info.url,
        status: null,
        ok: null,
        durationMs: null,
        error: null,
        reqBody: body !== undefined ? redactBody(body) : null,
        resType: null,
        resBody: null,
        t: t0,
      });
      state.inFlight += 1;
      let settled = false;
      const finish = (errMsg) => {
        if (settled) return;
        settled = true;
        state.inFlight = Math.max(0, state.inFlight - 1);
        entry.durationMs = Date.now() - t0;
        if (errMsg) {
          entry.error = errMsg;
        } else {
          entry.status = this.status;
          entry.ok = this.status >= 200 && this.status < 300;
          try {
            const ct = this.getResponseHeader('content-type') || '';
            entry.resType = ct.split(';')[0].trim() || null;
            if ((!this.responseType || this.responseType === 'text') && isTextType(ct)) {
              entry.resBody = redactBody(this.responseText);
            }
          } catch (_) {}
        }
      };
      this.addEventListener('load', () => finish(null));
      this.addEventListener('error', () => finish('network error'));
      this.addEventListener('abort', () => finish('aborted'));
      this.addEventListener('timeout', () => finish('timeout'));
      return origSend.apply(this, arguments);
    };
  }

  // Native dialogs kept untouched unless explicitly armed via `browser_dialog`.
  const nativeDialogs = { alert: window.alert, confirm: window.confirm, prompt: window.prompt };
  state.armDialog = (accept, promptText) => {
    state.dialogArmed = { accept, promptText };
    const record = (type, message, response) => {
      state.dialogs.push({ type, message: String(message ?? ''), response, t: Date.now() });
      if (state.dialogs.length > 50) state.dialogs.shift();
    };
    window.alert = (msg) => { record('alert', msg, null); };
    window.confirm = (msg) => { record('confirm', msg, accept); return accept; };
    window.prompt = (msg, def) => {
      const out = accept ? (promptText !== null ? promptText : String(def ?? '')) : null;
      record('prompt', msg, out);
      return out;
    };
  };
  state.resetDialog = () => {
    state.dialogArmed = null;
    window.alert = nativeDialogs.alert;
    window.confirm = nativeDialogs.confirm;
    window.prompt = nativeDialogs.prompt;
  };

  // Query selector searching open Shadow DOM roots when not found in document.
  const queryDeep = (sel, root = document) => {
    if (!sel) return null;
    let found = null;
    try { found = root.querySelector(sel); } catch (_) { return null; }
    if (found) return found;
    const stack = [root];
    while (stack.length) {
      const cur = stack.pop();
      const children = cur.querySelectorAll ? cur.querySelectorAll('*') : [];
      for (const el of children) {
        if (el.shadowRoot) {
          try { found = el.shadowRoot.querySelector(sel); } catch (_) {}
          if (found) return found;
          stack.push(el.shadowRoot);
        }
      }
    }
    return null;
  };
  state.queryDeep = queryDeep;
  state.resolve = (ref, sel) => (ref !== null && ref !== undefined ? state.refs.get(ref) : queryDeep(sel));

  const selector = (el) => {
    const parts = [];
    while (el && el.nodeType === 1 && parts.length < 7) {
      if (el.id) { parts.unshift('#' + CSS.escape(el.id)); break; }
      const testId = el.getAttribute('data-testid');
      if (testId) { parts.unshift(`${el.localName}[data-testid="${CSS.escape(testId)}"]`); break; }
      let part = el.localName;
      const parent = el.parentElement;
      if (parent) {
        const same = [...parent.children].filter((c) => c.localName === el.localName);
        if (same.length > 1) part += `:nth-of-type(${same.indexOf(el) + 1})`;
      }
      parts.unshift(part);
      el = parent;
    }
    return parts.join(' > ');
  };
  const describe = (el) => el.localName + (el.id ? '#' + el.id : '') +
    (el.classList.length ? '.' + [...el.classList].slice(0, 2).join('.') : '');
  state.selector = selector;
  state.describe = describe;
  window.__bencode = state;
})();"#;

/// Tells the app the page took the keyboard (`focused`), in every frame:
/// a press by the user, not one of `click`'s scripted ones.
pub const FOCUS: &str = r#"(() => {
  const say = (e) => {
    if (!e.isTrusted) return;
    try { window.webkit.messageHandlers.ipc.postMessage('{"type":"focused"}'); } catch (_) {}
  };
  window.addEventListener('mousedown', say, true);
  window.addEventListener('focus', say);
})();"#;

/// Starts (or stops) the element picker: the hovered element is outlined,
/// a click posts `picked` with its selector, markup, text and box, Esc
/// posts `pickCancelled`.
pub fn picker() -> String {
    format!(
        "{PRELUDE}\n{}",
        r#"(() => {
  const b = window.__bencode;
  if (b.picker) { b.picker(); return JSON.stringify({ stopped: true }); }
  const box = document.createElement('div');
  box.style.cssText = 'position:fixed;pointer-events:none;z-index:2147483647;border:2px solid #3b82f6;background:rgba(59,130,246,.15);border-radius:3px;display:none';
  const tag = document.createElement('div');
  tag.style.cssText = 'position:fixed;pointer-events:none;z-index:2147483647;background:#1d4ed8;color:#fff;font:11px ui-monospace,Menlo,monospace;padding:2px 6px;border-radius:4px;display:none;white-space:nowrap';
  document.documentElement.append(box, tag);
  let target = null;
  const move = (e) => {
    const el = document.elementFromPoint(e.clientX, e.clientY);
    if (!el || el === box || el === tag) return;
    target = el;
    const r = el.getBoundingClientRect();
    Object.assign(box.style, { display: 'block', left: r.left + 'px', top: r.top + 'px', width: r.width + 'px', height: r.height + 'px' });
    tag.textContent = b.describe(el);
    Object.assign(tag.style, { display: 'block', left: Math.max(0, r.left) + 'px', top: Math.max(0, r.top - 22) + 'px' });
  };
  const swallow = (e) => { e.preventDefault(); e.stopPropagation(); };
  const stop = () => {
    document.removeEventListener('mousemove', move, true);
    document.removeEventListener('mousedown', swallow, true);
    document.removeEventListener('mouseup', swallow, true);
    document.removeEventListener('click', click, true);
    document.removeEventListener('keydown', key, true);
    box.remove(); tag.remove();
    b.picker = null;
  };
  const click = (e) => {
    swallow(e);
    const el = target;
    stop();
    if (!el) { b.post({ type: 'pickCancelled' }); return; }
    const r = el.getBoundingClientRect();
    b.post({
      type: 'picked',
      selector: b.selector(el),
      html: el.outerHTML.slice(0, 4000),
      text: (el.innerText || '').trim().slice(0, 1000),
      rect: { x: r.left, y: r.top, width: r.width, height: r.height },
      viewport: { width: innerWidth, height: innerHeight },
      url: location.href,
      title: document.title,
    });
  };
  const key = (e) => {
    if (e.key !== 'Escape') return;
    swallow(e);
    stop();
    b.post({ type: 'pickCancelled' });
  };
  document.addEventListener('mousemove', move, true);
  document.addEventListener('mousedown', swallow, true);
  document.addEventListener('mouseup', swallow, true);
  document.addEventListener('click', click, true);
  document.addEventListener('keydown', key, true);
  b.picker = () => { stop(); b.post({ type: 'pickCancelled' }); };
  return JSON.stringify({ started: true });
})()"#
    )
}

/// The page as an agent reads it: title, address, viewport/scroll position,
/// visible text, and interactive elements (including open Shadow DOM and
/// `<label>` associations) numbered for `click` / `type` / `select` / `hover`.
/// Password values never leave the page.
pub fn outline(selector: Option<&str>, max_text: usize) -> String {
    let sel_js = serde_json::to_string(&selector.unwrap_or_default()).unwrap_or_default();
    format!(
        "{PRELUDE}\n(() => {{\n  const scopeSel = {sel_js};\n  const maxText = {max_text};\n{}\n}})()",
        r#"  const b = window.__bencode;
  b.refs = new Map();
  const root = scopeSel ? b.queryDeep(scopeSel) : document;
  if (!root) {
    return JSON.stringify({ ok: false, error: `No element matches selector "${scopeSel}".` });
  }
  const shown = (el) => {
    const r = el.getBoundingClientRect();
    if (r.width === 0 && r.height === 0) return false;
    const s = getComputedStyle(el);
    return s.visibility !== 'hidden' && s.display !== 'none';
  };
  const labelFor = (el) => {
    const labelledBy = el.getAttribute('aria-labelledby');
    if (labelledBy) {
      const t = labelledBy.split(/\s+/).map((id) => document.getElementById(id)?.innerText || '').join(' ').trim();
      if (t) return t;
    }
    if (el.labels && el.labels.length) {
      const t = [...el.labels].map((l) => l.innerText || '').join(' ').trim();
      if (t) return t;
    }
    if (el.id) {
      const lbl = document.querySelector(`label[for="${CSS.escape(el.id)}"]`);
      if (lbl && lbl.innerText) return lbl.innerText.trim();
    }
    const parentLabel = el.closest && el.closest('label');
    if (parentLabel && parentLabel !== el) return (parentLabel.innerText || '').trim();
    return '';
  };
  const name = (el) => (
    el.getAttribute('aria-label')
    || labelFor(el)
    || (el.localName === 'select' ? '' : el.innerText)
    || (el.type === 'password' ? '' : el.getAttribute('placeholder'))
    || el.getAttribute('title')
    || el.getAttribute('alt')
    || el.getAttribute('name')
    || ''
  ).trim().replace(/\s+/g, ' ').slice(0, 80);

  const query = 'a[href],button,input,textarea,select,summary,[role=button],[role=link],[role=tab],[role=menuitem],[role=checkbox],[role=radio],[role=switch],[role=combobox],[role=option],[contenteditable=""],[contenteditable=true],[onclick],[tabindex]:not([tabindex="-1"])';
  const collected = [];
  const walk = (node) => {
    if (!node || collected.length >= 300) return;
    if (node.nodeType === 1 && node.matches && node.matches(query)) {
      collected.push(node);
    }
    const list = node.querySelectorAll ? node.querySelectorAll('*') : [];
    for (const el of list) {
      if (collected.length >= 300) break;
      if (el.matches && el.matches(query)) collected.push(el);
      if (el.shadowRoot) walk(el.shadowRoot);
    }
  };
  walk(root);

  const seen = new Set();
  const lines = [];
  let n = 0;
  for (const el of collected) {
    if (seen.has(el)) continue;
    seen.add(el);
    if (!shown(el)) continue;
    n += 1;
    b.refs.set(n, el);
    const role = el.getAttribute('role') || el.localName;
    let line = `[${n}] ${role}`;
    if (el.localName === 'input') line += `[type=${el.type}]`;
    const label = name(el);
    if (label) line += ` "${label}"`;
    if (el.localName === 'a') line += ` -> ${el.getAttribute('href')}`;
    if (el.localName === 'select') {
      const selOpt = el.selectedOptions && el.selectedOptions[0];
      if (selOpt) line += ` value="${(selOpt.textContent || selOpt.value).trim().slice(0, 60)}"`;
      const opts = [...el.options].slice(0, 8).map((o) => (o.textContent || o.value).trim());
      if (opts.length) line += ` options=[${opts.join(', ')}${el.options.length > 8 ? ', …' : ''}]`;
    } else if (el.type === 'password') {
      // A password never leaves the page.
      if (el.value) line += ' (filled)';
    } else if ('value' in el && el.localName !== 'button' && el.type !== 'checkbox' && el.type !== 'radio' && el.value) {
      line += ` value="${String(el.value).slice(0, 80)}"`;
    }
    if (el.disabled || el.getAttribute('aria-disabled') === 'true') line += ' (disabled)';
    if (el.required || el.getAttribute('aria-required') === 'true') line += ' (required)';
    if (el.getAttribute('aria-invalid') === 'true') line += ' (invalid)';
    if (el.type === 'checkbox' || el.type === 'radio' || role === 'checkbox' || role === 'switch' || role === 'radio') {
      const checked = el.checked === true || el.getAttribute('aria-checked') === 'true';
      line += checked ? ' (checked)' : ' (unchecked)';
    }
    const exp = el.getAttribute('aria-expanded');
    if (exp === 'true') line += ' (expanded)';
    else if (exp === 'false') line += ' (collapsed)';
    if (el.getAttribute('aria-selected') === 'true') line += ' (selected)';
    if (el.getAttribute('aria-pressed') === 'true') line += ' (pressed)';
    lines.push(line);
  }
  const textNode = scopeSel ? root : document.body;
  const text = (textNode ? (textNode.innerText || '') : '').replace(/\n{3,}/g, '\n\n').trim();
  const maxY = Math.max(0, document.documentElement.scrollHeight - window.innerHeight);
  const maxX = Math.max(0, document.documentElement.scrollWidth - window.innerWidth);
  return JSON.stringify({
    ok: true,
    title: document.title,
    url: location.href,
    viewport: { width: window.innerWidth, height: window.innerHeight },
    scroll: { x: Math.round(window.scrollX), y: Math.round(window.scrollY), maxX: Math.round(maxX), maxY: Math.round(maxY) },
    text: text.length > maxText ? text.slice(0, maxText) + '\n…(truncated)' : text,
    elements: lines,
  });"#
    )
}

/// The element a tool names: a number from `outline`, else a CSS selector.
/// Defines `el`, or returns an error object.
pub(super) fn target(reference: Option<u64>, selector: Option<&str>) -> String {
    let selector = serde_json::to_string(&selector.unwrap_or_default()).unwrap_or_default();
    let reference = reference.map_or("null".to_string(), |n| n.to_string());
    format!(
        r#"const b = window.__bencode;
  const ref = {reference};
  const sel = {selector};
  const el = b.resolve(ref, sel);
  if (!el) return JSON.stringify({{ ok: false, error: ref !== null
    ? `No element [${{ref}}]; take a new browser_snapshot first.`
    : `No element matches ${{sel || '(no selector)'}}.` }});
  if (!el.isConnected) return JSON.stringify({{ ok: false, error: 'The element left the page; take a new browser_snapshot.' }});"#
    )
}

/// Clicks an element: supports double-click, right-click (`contextmenu`), and modifier keys.
pub fn click(
    reference: Option<u64>,
    selector: Option<&str>,
    double_click: bool,
    right_button: bool,
    modifiers: &[String],
) -> String {
    let mods_json = serde_json::to_string(modifiers).unwrap_or_else(|_| "[]".into());
    format!(
        "{PRELUDE}\n(() => {{ try {{\n  {}\n  const dbl = {double_click};\n  const right = {right_button};\n  const mods = new Set({mods_json}.map((m) => String(m).toLowerCase()));\n{}\n}} catch (e) {{ return JSON.stringify({{ ok: false, error: String(e) }}); }} }})()",
        target(reference, selector),
        r#"  el.scrollIntoView({ block: 'center', inline: 'center' });
  const r = el.getBoundingClientRect();
  const at = {
    bubbles: true,
    cancelable: true,
    view: window,
    clientX: r.left + r.width / 2,
    clientY: r.top + r.height / 2,
    button: right ? 2 : 0,
    buttons: right ? 2 : 1,
    shiftKey: mods.has('shift'),
    ctrlKey: mods.has('control') || mods.has('ctrl'),
    altKey: mods.has('alt') || mods.has('option'),
    metaKey: mods.has('meta') || mods.has('command') || mods.has('cmd'),
  };
  const fireOnce = (detail) => {
    const ev = { ...at, detail };
    el.dispatchEvent(new PointerEvent('pointerdown', ev));
    el.dispatchEvent(new MouseEvent('mousedown', ev));
    if (typeof el.focus === 'function') el.focus();
    el.dispatchEvent(new PointerEvent('pointerup', ev));
    el.dispatchEvent(new MouseEvent('mouseup', ev));
    if (right) {
      el.dispatchEvent(new MouseEvent('contextmenu', ev));
    } else if (mods.size > 0) {
      el.dispatchEvent(new MouseEvent('click', ev));
    } else {
      el.click();
    }
  };
  fireOnce(1);
  if (dbl && !right) {
    fireOnce(2);
    el.dispatchEvent(new MouseEvent('dblclick', { ...at, detail: 2 }));
  }
  const verb = right ? 'Right-clicked' : (dbl ? 'Double-clicked' : 'Clicked');
  return JSON.stringify({ ok: true, clicked: `${verb} ${b.describe(el)}` });"#
    )
}

pub fn type_text(
    reference: Option<u64>,
    selector: Option<&str>,
    text: &str,
    submit: bool,
    append: bool,
) -> String {
    let text = serde_json::to_string(text).unwrap_or_default();
    format!(
        "{PRELUDE}\n(() => {{ try {{\n  {}\n  const text = {text};\n  const submit = {submit};\n  const append = {append};\n{}\n}} catch (e) {{ return JSON.stringify({{ ok: false, error: String(e) }}); }} }})()",
        target(reference, selector),
        r#"  el.scrollIntoView({ block: 'center' });
  el.focus();
  if (el.isContentEditable) {
    const next = append ? (el.textContent || '') + text : text;
    el.textContent = next;
    el.dispatchEvent(new InputEvent('input', { bubbles: true, data: text }));
  } else if ('value' in el) {
    // The prototype's setter, so React and friends see the change.
    const next = append ? String(el.value || '') + text : text;
    const proto = Object.getPrototypeOf(el);
    const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set;
    if (setter) setter.call(el, next); else el.value = next;
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  } else {
    return JSON.stringify({ ok: false, error: `${b.describe(el)} does not take text.` });
  }
  if (submit) {
    const enter = { key: 'Enter', code: 'Enter', keyCode: 13, which: 13, bubbles: true, cancelable: true };
    const go = el.dispatchEvent(new KeyboardEvent('keydown', enter));
    el.dispatchEvent(new KeyboardEvent('keyup', enter));
    if (go && el.form) {
      if (typeof el.form.requestSubmit === 'function') el.form.requestSubmit(); else el.form.submit();
    }
  }
  return JSON.stringify({ ok: true, typed: b.describe(el) });"#
    )
}

/// A key (or modifier combo like `Control+Enter`, `Shift+Tab`, `Meta+k`)
/// pressed on the focused element, or on `reference`/`selector` when given.
pub fn press_key(key: &str, reference: Option<u64>, selector: Option<&str>) -> String {
    let key = serde_json::to_string(key).unwrap_or_default();
    let ref_js = reference.map_or("null".to_string(), |n| n.to_string());
    let sel_js = serde_json::to_string(&selector.unwrap_or_default()).unwrap_or_default();
    format!(
        r#"{PRELUDE}
(() => {{
  const b = window.__bencode;
  const raw = {key};
  const ref = {ref_js};
  const sel = {sel_js};
  if (ref !== null || sel) {{
    const target = b.resolve(ref, sel);
    if (!target || !target.isConnected) {{
      return JSON.stringify({{ ok: false, error: 'Target element not found; take a new browser_snapshot.' }});
    }}
    if (typeof target.focus === 'function') target.focus();
  }}
  const parts = raw.split('+');
  const mainKey = parts.pop() || raw;
  const mods = new Set(parts.map((p) => p.trim().toLowerCase()));
  const keyName = mainKey === 'Space' ? ' ' : mainKey;
  const el = document.activeElement || document.body;
  const codes = {{ Enter: 13, Escape: 27, Tab: 9, Backspace: 8, Delete: 46, ' ': 32, ArrowLeft: 37, ArrowUp: 38, ArrowRight: 39, ArrowDown: 40 }};
  const code = codes[keyName] || (keyName.length === 1 ? keyName.toUpperCase().charCodeAt(0) : 0);
  const init = {{
    key: keyName,
    code: keyName === ' ' ? 'Space' : (keyName.length === 1 && /[a-z]/i.test(keyName) ? 'Key' + keyName.toUpperCase() : keyName),
    keyCode: code,
    which: code,
    bubbles: true,
    cancelable: true,
    shiftKey: mods.has('shift'),
    ctrlKey: mods.has('control') || mods.has('ctrl'),
    altKey: mods.has('alt') || mods.has('option'),
    metaKey: mods.has('meta') || mods.has('command') || mods.has('cmd'),
  }};
  const go = el.dispatchEvent(new KeyboardEvent('keydown', init));
  el.dispatchEvent(new KeyboardEvent('keypress', init));
  el.dispatchEvent(new KeyboardEvent('keyup', init));
  let effect = '';
  const noMods = !init.ctrlKey && !init.altKey && !init.metaKey;
  if (go && noMods && !init.shiftKey && keyName === 'Enter') {{
    if (el.matches('button,a[href],summary,[role=button],[role=link],input[type=submit],input[type=button]')) {{
      el.click();
      effect = '; clicked ' + b.describe(el);
    }} else if (el.form && el.localName !== 'textarea') {{
      if (typeof el.form.requestSubmit === 'function') el.form.requestSubmit(); else el.form.submit();
      effect = '; submitted the form';
    }}
  }} else if (go && noMods && !init.shiftKey && keyName === ' ') {{
    if (el.matches('button,summary,[role=button],[role=checkbox],[role=switch],input[type=checkbox],input[type=radio]')) {{
      el.click();
      effect = '; toggled ' + b.describe(el);
    }}
  }} else if (go && noMods && keyName === 'Tab') {{
    const stops = [...document.querySelectorAll('a[href],button,input,select,textarea,[tabindex]')]
      .filter((e) => e.tabIndex >= 0 && !e.disabled && e.getClientRects().length);
    const idx = stops.indexOf(el);
    const next = init.shiftKey
      ? (idx > 0 ? stops[idx - 1] : stops[stops.length - 1])
      : (stops[idx + 1] || stops[0]);
    if (next) {{ next.focus(); effect = '; focus moved to ' + b.describe(next); }}
  }}
  return JSON.stringify({{ ok: true, target: b.describe(el), effect }});
}})()"#
    )
}

/// Runs an expression. A promise is settled into `__bencode.evals[id]`,
/// which `eval_result` reads back.
pub fn evaluate(expression: &str, id: u64, inline: bool) -> String {
    let run = if inline {
        format!("() => ({expression}\n)")
    } else {
        let quoted = serde_json::to_string(expression).unwrap_or_default();
        format!("() => (0, eval)({quoted})")
    };
    format!(
        r#"{PRELUDE}
((run) => {{
  const b = window.__bencode;
  b.evals = b.evals || {{}};
  const why = (e) => (e && e.message ? `${{e.name}}: ${{e.message}}` : String(e));
  const show = (v) => {{
    if (v === undefined) return 'undefined';
    if (v instanceof Element) return v.outerHTML.slice(0, 4000);
    try {{ return JSON.stringify(v, null, 2); }} catch (_) {{ return String(v); }}
  }};
  try {{
    const value = run();
    if (value && typeof value.then === 'function') {{
      b.evals[{id}] = null;
      value.then(
        (v) => {{ b.evals[{id}] = {{ ok: true, value: show(v) }}; }},
        (e) => {{ b.evals[{id}] = {{ ok: false, error: why(e) }}; }});
      return JSON.stringify({{ pending: true }});
    }}
    return JSON.stringify({{ ok: true, value: show(value) }});
  }} catch (e) {{
    return JSON.stringify({{ ok: false, error: why(e) }});
  }}
}})({run})"#
    )
}

/// A settled promise from `evaluate`, or `{ pending: true }`.
pub fn eval_result(id: u64) -> String {
    format!(
        r#"(() => {{
  const evals = (window.__bencode && window.__bencode.evals) || {{}};
  const done = evals[{id}];
  if (!done) return JSON.stringify({{ pending: true }});
  delete evals[{id}];
  return JSON.stringify(done);
}})()"#
    )
}

/// The console messages kept since the page loaded, optionally filtered by
/// level, substring search, and limit; `clear` empties the buffer after reading.
pub fn console(
    clear: bool,
    level: Option<&str>,
    search: Option<&str>,
    limit: Option<usize>,
) -> String {
    let level_js = serde_json::to_string(&level.unwrap_or_default()).unwrap_or_default();
    let search_js = serde_json::to_string(&search.unwrap_or_default()).unwrap_or_default();
    let limit_js = limit.map_or("0".to_string(), |n| n.to_string());
    format!(
        r#"{PRELUDE}
(() => {{
  const logs = window.__bencode.logs;
  const level = {level_js}.toLowerCase();
  const search = {search_js}.toLowerCase();
  const limit = {limit_js};
  let filtered = logs.slice();
  if (level === 'errors') filtered = filtered.filter((l) => l.level === 'error' || l.level === 'warn');
  else if (level) filtered = filtered.filter((l) => l.level === level);
  if (search) filtered = filtered.filter((l) => String(l.text).toLowerCase().includes(search));
  if (limit > 0) filtered = filtered.slice(-limit);
  const out = filtered.map((l) => `[${{l.level}}] ${{l.text}}`);
  if ({clear}) logs.length = 0;
  return JSON.stringify({{ ok: true, lines: out }});
}})()"#
    )
}
