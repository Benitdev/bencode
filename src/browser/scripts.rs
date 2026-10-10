//! The scripts BenCode runs in a browser page. `PRELUDE` installs
//! `window.__bencode` (console capture, address changes, selectors); every
//! other script starts with it, since a page loaded before it (an error
//! page, `about:blank`) has none. Each script evaluates to a JSON string.

/// Console capture, history hooks and the helpers the other scripts use.
/// Idempotent.
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
  window.__bencode = { logs, post, selector, describe, refs: new Map(), picker: null };
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

/// The page as an agent reads it: title, address, visible text, and the
/// interactive elements numbered for `click` / `type` (`refs`).
pub fn outline() -> String {
    format!(
        "{PRELUDE}\n{}",
        r#"(() => {
  const b = window.__bencode;
  b.refs = new Map();
  const shown = (el) => {
    const r = el.getBoundingClientRect();
    if (r.width === 0 && r.height === 0) return false;
    const s = getComputedStyle(el);
    return s.visibility !== 'hidden' && s.display !== 'none';
  };
  const name = (el) => (el.getAttribute('aria-label') || el.innerText ||
    (el.type === 'password' ? '' : el.value) || el.getAttribute('placeholder') || el.getAttribute('title') || el.getAttribute('alt') || '')
    .trim().replace(/\s+/g, ' ').slice(0, 80);
  const query = 'a[href],button,input,textarea,select,summary,[role=button],[role=link],[role=tab],[role=menuitem],[role=checkbox],[role=switch],[contenteditable=""],[contenteditable=true],[onclick],[tabindex]:not([tabindex="-1"])';
  const lines = [];
  let n = 0;
  for (const el of document.querySelectorAll(query)) {
    if (n >= 300) break;
    if (!shown(el)) continue;
    n += 1;
    b.refs.set(n, el);
    const role = el.getAttribute('role') || el.localName;
    let line = `[${n}] ${role}`;
    if (el.localName === 'input') line += `[type=${el.type}]`;
    const label = name(el);
    if (label) line += ` "${label}"`;
    if (el.localName === 'a') line += ` -> ${el.getAttribute('href')}`;
    // A password never leaves the page.
    if (el.type === 'password') { if (el.value) line += ' (filled)'; }
    else if ('value' in el && el.localName !== 'button' && el.value) line += ` value="${String(el.value).slice(0, 80)}"`;
    if (el.disabled) line += ' (disabled)';
    if (el.checked) line += ' (checked)';
    lines.push(line);
  }
  const text = (document.body ? document.body.innerText : '').replace(/\n{3,}/g, '\n\n').trim();
  return JSON.stringify({
    title: document.title,
    url: location.href,
    text: text.length > 12000 ? text.slice(0, 12000) + '\n…(truncated)' : text,
    elements: lines,
  });
})()"#
    )
}

/// The element a tool names: a number from `outline`, else a CSS selector.
/// Defines `el`, or returns an error object.
fn target(reference: Option<u64>, selector: Option<&str>) -> String {
    let selector = serde_json::to_string(&selector.unwrap_or_default()).unwrap_or_default();
    let reference = reference.map_or("null".to_string(), |n| n.to_string());
    format!(
        r#"const b = window.__bencode;
  const ref = {reference};
  const sel = {selector};
  const el = ref !== null ? b.refs.get(ref) : (sel ? document.querySelector(sel) : null);
  if (!el) return JSON.stringify({{ ok: false, error: ref !== null
    ? `No element [${{ref}}]; take a new browser_snapshot first.`
    : `No element matches ${{sel || '(no selector)'}}.` }});
  if (!el.isConnected) return JSON.stringify({{ ok: false, error: 'The element left the page; take a new browser_snapshot.' }});"#
    )
}

pub fn click(reference: Option<u64>, selector: Option<&str>) -> String {
    format!(
        "{PRELUDE}\n(() => {{ try {{\n  {}\n{}\n}} catch (e) {{ return JSON.stringify({{ ok: false, error: String(e) }}); }} }})()",
        target(reference, selector),
        r#"  el.scrollIntoView({ block: 'center', inline: 'center' });
  const r = el.getBoundingClientRect();
  const at = { bubbles: true, cancelable: true, view: window, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2 };
  el.dispatchEvent(new PointerEvent('pointerdown', at));
  el.dispatchEvent(new MouseEvent('mousedown', at));
  if (typeof el.focus === 'function') el.focus();
  el.dispatchEvent(new PointerEvent('pointerup', at));
  el.dispatchEvent(new MouseEvent('mouseup', at));
  el.click();
  return JSON.stringify({ ok: true, clicked: b.describe(el) });"#
    )
}

pub fn type_text(
    reference: Option<u64>,
    selector: Option<&str>,
    text: &str,
    submit: bool,
) -> String {
    let text = serde_json::to_string(text).unwrap_or_default();
    format!(
        "{PRELUDE}\n(() => {{ try {{\n  {}\n  const text = {text};\n  const submit = {submit};\n{}\n}} catch (e) {{ return JSON.stringify({{ ok: false, error: String(e) }}); }} }})()",
        target(reference, selector),
        r#"  el.scrollIntoView({ block: 'center' });
  el.focus();
  if (el.isContentEditable) {
    el.textContent = text;
    el.dispatchEvent(new InputEvent('input', { bubbles: true, data: text }));
  } else if ('value' in el) {
    // The prototype's setter, so React and friends see the change.
    const proto = Object.getPrototypeOf(el);
    const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set;
    if (setter) setter.call(el, text); else el.value = text;
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

/// A key pressed on the focused element (`Enter`, `Escape`, `Tab`, …). A
/// scripted key is not trusted, so the browser does nothing of its own with
/// it: what Enter and Tab would do is done here, and told in `effect`.
pub fn press_key(key: &str) -> String {
    let key = serde_json::to_string(key).unwrap_or_default();
    format!(
        r#"{PRELUDE}
(() => {{
  const b = window.__bencode;
  const key = {key};
  const el = document.activeElement || document.body;
  const codes = {{ Enter: 13, Escape: 27, Tab: 9, Backspace: 8, Delete: 46, ' ': 32, ArrowLeft: 37, ArrowUp: 38, ArrowRight: 39, ArrowDown: 40 }};
  const code = codes[key] || 0;
  const init = {{ key, code: key === ' ' ? 'Space' : key, keyCode: code, which: code, bubbles: true, cancelable: true }};
  const go = el.dispatchEvent(new KeyboardEvent('keydown', init));
  el.dispatchEvent(new KeyboardEvent('keypress', init));
  el.dispatchEvent(new KeyboardEvent('keyup', init));
  let effect = '';
  if (go && key === 'Enter') {{
    if (el.matches('button,a[href],summary,[role=button],[role=link],input[type=submit],input[type=button]')) {{
      el.click();
      effect = '; clicked ' + b.describe(el);
    }} else if (el.form && el.localName !== 'textarea') {{
      if (typeof el.form.requestSubmit === 'function') el.form.requestSubmit(); else el.form.submit();
      effect = '; submitted the form';
    }}
  }} else if (go && key === 'Tab') {{
    const stops = [...document.querySelectorAll('a[href],button,input,select,textarea,[tabindex]')]
      .filter((e) => e.tabIndex >= 0 && !e.disabled && e.getClientRects().length);
    const next = stops[stops.indexOf(el) + 1] || stops[0];
    if (next) {{ next.focus(); effect = '; focus moved to ' + b.describe(next); }}
  }}
  return JSON.stringify({{ ok: true, target: b.describe(el), effect }});
}})()"#
    )
}

/// Runs an expression. A promise is settled into `__bencode.evals[id]`,
/// which `eval_result` reads back.
///
/// `inline` writes the expression into the script itself, which a page's
/// Content-Security-Policy does not stop as it stops `eval`; a syntax error
/// (statements are one) then fails the whole script with nothing to read,
/// and the caller runs it again through `eval`, which says what is wrong.
pub fn evaluate(expression: &str, id: u64, inline: bool) -> String {
    // Passed in from outside the function below, so the expression sees the
    // page's names and none of this script's.
    let run = if inline {
        // The line break ends a trailing `//` comment.
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
  // WebKit's `stack` leaves the message out.
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

/// The console messages kept since the page loaded; `clear` empties them.
pub fn console(clear: bool) -> String {
    format!(
        r#"{PRELUDE}
(() => {{
  const logs = window.__bencode.logs;
  const out = logs.map((l) => `[${{l.level}}] ${{l.text}}`);
  if ({clear}) logs.length = 0;
  return JSON.stringify({{ ok: true, lines: out }});
}})()"#
    )
}

/// Whether `text` shows on the page.
pub fn has_text(text: &str) -> String {
    let text = serde_json::to_string(text).unwrap_or_default();
    format!(
        "JSON.stringify({{ found: !!(document.body && document.body.innerText.includes({text})) }})"
    )
}
