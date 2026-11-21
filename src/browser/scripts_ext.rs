//! Extended browser automation and inspection scripts for `bencode-browser`
//! MCP: hover, select, batch form fill, scroll, drag-and-drop, element rect
//! (for cropped screenshots), network request inspection, DOM/CSS inspection,
//! web storage/cookies, JS dialog handling, and rich wait predicates.
//!
//! Every script starts with [`super::scripts::PRELUDE`] and evaluates to a
//! JSON string.

use serde_json::Value;

use super::scripts::{PRELUDE, target};

/// Hovers an element (`pointerover`, `pointerenter`, `mouseover`, `mouseenter`, `mousemove`).
pub fn hover(reference: Option<u64>, selector: Option<&str>) -> String {
    format!(
        "{PRELUDE}\n(() => {{ try {{\n  {}\n{}\n}} catch (e) {{ return JSON.stringify({{ ok: false, error: String(e) }}); }} }})()",
        target(reference, selector),
        r#"  el.scrollIntoView({ block: 'center', inline: 'center' });
  const r = el.getBoundingClientRect();
  const at = { bubbles: true, cancelable: true, view: window, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2 };
  el.dispatchEvent(new PointerEvent('pointerover', at));
  el.dispatchEvent(new PointerEvent('pointerenter', { ...at, bubbles: false }));
  el.dispatchEvent(new MouseEvent('mouseover', at));
  el.dispatchEvent(new MouseEvent('mouseenter', { ...at, bubbles: false }));
  el.dispatchEvent(new MouseEvent('mousemove', at));
  return JSON.stringify({ ok: true, hovered: b.describe(el) });"#
    )
}

/// Selects one or more options in a `<select>` element by value or visible label.
pub fn select_option(reference: Option<u64>, selector: Option<&str>, values: &[String]) -> String {
    let values_json = serde_json::to_string(values).unwrap_or_else(|_| "[]".into());
    format!(
        "{PRELUDE}\n(() => {{ try {{\n  {}\n  const wanted = {values_json};\n{}\n}} catch (e) {{ return JSON.stringify({{ ok: false, error: String(e) }}); }} }})()",
        target(reference, selector),
        r#"  if (el.localName !== 'select') {
    return JSON.stringify({ ok: false, error: `${b.describe(el)} is not a <select> element.` });
  }
  const opts = [...el.options];
  const findOpt = (w) => {
    const low = String(w).trim().toLowerCase();
    return opts.find((o) => o.value === w)
      || opts.find((o) => o.textContent.trim() === w)
      || opts.find((o) => o.value.toLowerCase() === low)
      || opts.find((o) => o.textContent.trim().toLowerCase() === low);
  };
  const chosen = [];
  for (const w of wanted) {
    const match = findOpt(w);
    if (!match) {
      const choices = opts.map((o) => o.value === o.textContent.trim() ? o.value : `${o.textContent.trim()} (${o.value})`);
      return JSON.stringify({ ok: false, error: `No option matching "${w}" in ${b.describe(el)}; choices: ${choices.join(', ')}` });
    }
    chosen.push(match);
  }
  if (!el.multiple && chosen.length > 1) {
    return JSON.stringify({ ok: false, error: `${b.describe(el)} does not allow multiple selections.` });
  }
  el.scrollIntoView({ block: 'center' });
  el.focus();
  if (el.multiple) {
    const set = new Set(chosen);
    for (const o of opts) o.selected = set.has(o);
  } else if (chosen[0]) {
    const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value')?.set;
    if (setter) setter.call(el, chosen[0].value); else el.value = chosen[0].value;
  }
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  const labels = chosen.map((o) => o.textContent.trim() || o.value);
  return JSON.stringify({ ok: true, selected: labels, target: b.describe(el) });"#
    )
}

/// Fills multiple form fields in a single round-trip.
pub fn fill_form(fields: &Value, submit: bool) -> String {
    let fields_json = serde_json::to_string(fields).unwrap_or_else(|_| "[]".into());
    format!(
        r#"{PRELUDE}
(() => {{
  try {{
    const b = window.__bencode;
    const fields = {fields_json};
    const submit = {submit};
    if (!Array.isArray(fields) || fields.length === 0) {{
      return JSON.stringify({{ ok: false, error: 'fields must be a non-empty array.' }});
    }}
    const results = [];
    let lastForm = null;
    for (const f of fields) {{
      const ref = typeof f.ref === 'number' ? f.ref : null;
      const sel = typeof f.selector === 'string' ? f.selector : '';
      const el = b.resolve(ref, sel);
      if (!el || !el.isConnected) {{
        return JSON.stringify({{ ok: false, error: ref !== null
          ? `No element [${{ref}}]; take a new browser_snapshot first.`
          : `No element matches ${{sel || '(no selector)'}}.` }});
      }}
      if (el.form) lastForm = el.form;
      el.scrollIntoView({{ block: 'center' }});
      el.focus();
      if (el.localName === 'select') {{
        const wanted = Array.isArray(f.values) ? f.values : [f.value ?? f.select ?? ''];
        const opts = [...el.options];
        const findOpt = (w) => {{
          const s = String(w);
          const low = s.trim().toLowerCase();
          return opts.find((o) => o.value === s)
            || opts.find((o) => o.textContent.trim() === s)
            || opts.find((o) => o.value.toLowerCase() === low)
            || opts.find((o) => o.textContent.trim().toLowerCase() === low);
        }};
        const chosen = [];
        for (const w of wanted) {{
          const m = findOpt(w);
          if (!m) return JSON.stringify({{ ok: false, error: `No option "${{w}}" in ${{b.describe(el)}}.` }});
          chosen.push(m);
        }}
        if (el.multiple) {{
          const set = new Set(chosen);
          for (const o of opts) o.selected = set.has(o);
        }} else if (chosen[0]) {{
          const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value')?.set;
          if (setter) setter.call(el, chosen[0].value); else el.value = chosen[0].value;
        }}
        el.dispatchEvent(new Event('input', {{ bubbles: true }}));
        el.dispatchEvent(new Event('change', {{ bubbles: true }}));
        results.push(`selected ${{chosen.map((o) => o.textContent.trim() || o.value).join(', ')}} in ${{b.describe(el)}}`);
      }} else if (el.localName === 'input' && (el.type === 'checkbox' || el.type === 'radio')) {{
        const wantChecked = typeof f.checked === 'boolean' ? f.checked : Boolean(f.value ?? true);
        if (el.checked !== wantChecked) {{
          el.click();
          if (el.checked !== wantChecked) {{
            el.checked = wantChecked;
            el.dispatchEvent(new Event('input', {{ bubbles: true }}));
            el.dispatchEvent(new Event('change', {{ bubbles: true }}));
          }}
        }}
        results.push(`${{wantChecked ? 'checked' : 'unchecked'}} ${{b.describe(el)}}`);
      }} else if (el.isContentEditable) {{
        const val = String(f.value ?? '');
        el.textContent = val;
        el.dispatchEvent(new InputEvent('input', {{ bubbles: true, data: val }}));
        results.push(`filled ${{b.describe(el)}}`);
      }} else if ('value' in el) {{
        const val = String(f.value ?? '');
        const proto = Object.getPrototypeOf(el);
        const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set;
        if (setter) setter.call(el, val); else el.value = val;
        el.dispatchEvent(new Event('input', {{ bubbles: true }}));
        el.dispatchEvent(new Event('change', {{ bubbles: true }}));
        results.push(`filled ${{b.describe(el)}}`);
      }} else {{
        return JSON.stringify({{ ok: false, error: `${{b.describe(el)}} is not an editable form control.` }});
      }}
    }}
    if (submit && lastForm) {{
      if (typeof lastForm.requestSubmit === 'function') lastForm.requestSubmit(); else lastForm.submit();
      results.push('submitted form');
    }}
    return JSON.stringify({{ ok: true, results }});
  }} catch (e) {{
    return JSON.stringify({{ ok: false, error: String(e) }});
  }}
}})()"#
    )
}

/// Scrolls the page or a specific element by direction/amount, or brings an element into view.
pub fn scroll(
    reference: Option<u64>,
    selector: Option<&str>,
    direction: Option<&str>,
    amount: Option<f64>,
    into_view: bool,
) -> String {
    let ref_js = reference.map_or("null".to_string(), |n| n.to_string());
    let sel_js = serde_json::to_string(&selector.unwrap_or_default()).unwrap_or_default();
    let dir_js = serde_json::to_string(&direction.unwrap_or("down")).unwrap_or_default();
    let amount_js = amount.map_or("null".to_string(), |a| a.to_string());
    format!(
        r#"{PRELUDE}
(() => {{
  try {{
    const b = window.__bencode;
    const ref = {ref_js};
    const sel = {sel_js};
    const dir = {dir_js};
    const amount = {amount_js};
    const intoView = {into_view};
    const hasTarget = ref !== null || !!sel;
    let el = null;
    if (hasTarget) {{
      el = b.resolve(ref, sel);
      if (!el || !el.isConnected) {{
        return JSON.stringify({{ ok: false, error: ref !== null
          ? `No element [${{ref}}]; take a new browser_snapshot first.`
          : `No element matches ${{sel}}.` }});
      }}
      if (intoView) {{
        el.scrollIntoView({{ block: 'center', inline: 'center' }});
        const r = el.getBoundingClientRect();
        return JSON.stringify({{
          ok: true,
          summary: `Scrolled ${{b.describe(el)}} into view (x=${{Math.round(r.left)}}, y=${{Math.round(r.top)}}).`,
        }});
      }}
    }}
    const isWin = !el;
    const stepY = amount !== null ? amount : Math.round((isWin ? window.innerHeight : el.clientHeight) * 0.8);
    const stepX = amount !== null ? amount : Math.round((isWin ? window.innerWidth : el.clientWidth) * 0.8);
    const maxY = isWin
      ? Math.max(0, document.documentElement.scrollHeight - window.innerHeight)
      : Math.max(0, el.scrollHeight - el.clientHeight);
    const maxX = isWin
      ? Math.max(0, document.documentElement.scrollWidth - window.innerWidth)
      : Math.max(0, el.scrollWidth - el.clientWidth);
    const move = (dx, dy, top, left) => {{
      const opts = {{ behavior: 'instant' }};
      if (top !== undefined || left !== undefined) {{
        if (top !== undefined) opts.top = top;
        if (left !== undefined) opts.left = left;
        (isWin ? window : el).scrollTo(opts);
      }} else {{
        (isWin ? window : el).scrollBy({{ left: dx, top: dy, behavior: 'instant' }});
      }}
    }};
    switch (dir) {{
      case 'down': move(0, stepY); break;
      case 'up': move(0, -stepY); break;
      case 'right': move(stepX, 0); break;
      case 'left': move(-stepX, 0); break;
      case 'top': move(0, 0, 0, undefined); break;
      case 'bottom': move(0, 0, maxY, undefined); break;
      default: return JSON.stringify({{ ok: false, error: `Unknown direction ${{dir}}; use up, down, left, right, top or bottom.` }});
    }}
    const curX = Math.round(isWin ? window.scrollX : el.scrollLeft);
    const curY = Math.round(isWin ? window.scrollY : el.scrollTop);
    const who = isWin ? 'page' : b.describe(el);
    return JSON.stringify({{
      ok: true,
      summary: `Scrolled ${{who}} ${{dir}} -> scroll=(${{curX}}/${{Math.round(maxX)}}, ${{curY}}/${{Math.round(maxY)}}).`,
    }});
  }} catch (e) {{
    return JSON.stringify({{ ok: false, error: String(e) }});
  }}
}})()"#
    )
}

/// Drags from a source element to a target element (pointer + mouse + HTML5 DragEvent).
pub fn drag(
    from_ref: Option<u64>,
    from_sel: Option<&str>,
    to_ref: Option<u64>,
    to_sel: Option<&str>,
) -> String {
    let f_ref = from_ref.map_or("null".to_string(), |n| n.to_string());
    let f_sel = serde_json::to_string(&from_sel.unwrap_or_default()).unwrap_or_default();
    let t_ref = to_ref.map_or("null".to_string(), |n| n.to_string());
    let t_sel = serde_json::to_string(&to_sel.unwrap_or_default()).unwrap_or_default();
    format!(
        r#"{PRELUDE}
(() => {{
  try {{
    const b = window.__bencode;
    const src = b.resolve({f_ref}, {f_sel});
    if (!src || !src.isConnected) return JSON.stringify({{ ok: false, error: 'Source element not found; take a new browser_snapshot.' }});
    const dst = b.resolve({t_ref}, {t_sel});
    if (!dst || !dst.isConnected) return JSON.stringify({{ ok: false, error: 'Target element not found; take a new browser_snapshot.' }});
    src.scrollIntoView({{ block: 'center', inline: 'center' }});
    const sr = src.getBoundingClientRect();
    const dr = dst.getBoundingClientRect();
    const sx = sr.left + sr.width / 2, sy = sr.top + sr.height / 2;
    const dx = dr.left + dr.width / 2, dy = dr.top + dr.height / 2;
    const dt = typeof DataTransfer === 'function' ? new DataTransfer() : null;
    const mkMouse = (type, x, y) => new MouseEvent(type, {{ bubbles: true, cancelable: true, view: window, clientX: x, clientY: y, button: 0, buttons: 1 }});
    const mkPtr = (type, x, y) => new PointerEvent(type, {{ bubbles: true, cancelable: true, view: window, clientX: x, clientY: y, button: 0, buttons: 1, pointerId: 1, isPrimary: true }});
    const mkDrag = (type, x, y) => new DragEvent(type, {{ bubbles: true, cancelable: true, clientX: x, clientY: y, dataTransfer: dt }});
    src.dispatchEvent(mkPtr('pointerdown', sx, sy));
    src.dispatchEvent(mkMouse('mousedown', sx, sy));
    src.dispatchEvent(mkDrag('dragstart', sx, sy));
    dst.dispatchEvent(mkPtr('pointermove', dx, dy));
    dst.dispatchEvent(mkMouse('mousemove', dx, dy));
    dst.dispatchEvent(mkDrag('dragenter', dx, dy));
    dst.dispatchEvent(mkDrag('dragover', dx, dy));
    dst.dispatchEvent(mkDrag('drop', dx, dy));
    src.dispatchEvent(mkDrag('dragend', dx, dy));
    dst.dispatchEvent(mkPtr('pointerup', dx, dy));
    dst.dispatchEvent(mkMouse('mouseup', dx, dy));
    return JSON.stringify({{ ok: true, summary: `Dragged ${{b.describe(src)}} to ${{b.describe(dst)}}.` }});
  }} catch (e) {{
    return JSON.stringify({{ ok: false, error: String(e) }});
  }}
}})()"#
    )
}

/// Scrolls an element into view and returns its bounding box and viewport size (for cropped screenshots).
pub fn element_rect(reference: Option<u64>, selector: Option<&str>) -> String {
    format!(
        "{PRELUDE}\n(() => {{ try {{\n  {}\n{}\n}} catch (e) {{ return JSON.stringify({{ ok: false, error: String(e) }}); }} }})()",
        target(reference, selector),
        r#"  el.scrollIntoView({ block: 'center', inline: 'center' });
  const r = el.getBoundingClientRect();
  return JSON.stringify({
    ok: true,
    described: b.describe(el),
    rect: { x: r.left, y: r.top, width: r.width, height: r.height },
    viewport: { width: window.innerWidth, height: window.innerHeight },
  });"#
    )
}

/// Lists captured `fetch`/`xhr` requests (and optionally static performance resources).
pub fn network_list(
    url_filter: Option<&str>,
    method_filter: Option<&str>,
    failed_only: bool,
    include_resources: bool,
    limit: usize,
    clear: bool,
) -> String {
    let url_f = serde_json::to_string(&url_filter.unwrap_or_default()).unwrap_or_default();
    let method_f = serde_json::to_string(&method_filter.unwrap_or_default()).unwrap_or_default();
    format!(
        r#"{PRELUDE}
(() => {{
  const b = window.__bencode;
  const urlF = {url_f}.toLowerCase();
  const methodF = {method_f}.toUpperCase();
  const failedOnly = {failed_only};
  const includeResources = {include_resources};
  const limit = {limit};
  let items = b.net.slice();
  if (urlF) items = items.filter((r) => String(r.url || '').toLowerCase().includes(urlF));
  if (methodF) items = items.filter((r) => String(r.method || '').toUpperCase() === methodF);
  if (failedOnly) items = items.filter((r) => !!r.error || (typeof r.status === 'number' && r.status >= 400));
  const total = items.length;
  items = items.slice(-limit);
  const lines = items.map((r) => {{
    const st = r.error ? `ERR(${{r.error}})` : (r.status === null ? 'pending' : String(r.status));
    const ms = typeof r.durationMs === 'number' ? ` ${{r.durationMs}}ms` : '';
    const ct = r.resType ? ` [${{r.resType}}]` : '';
    return `#${{r.id}} [${{r.kind}}] ${{r.method}} ${{st}}${{ms}}${{ct}} ${{r.url}}`;
  }});
  let resources = [];
  if (includeResources && typeof performance !== 'undefined' && performance.getEntriesByType) {{
    let entries = performance.getEntriesByType('resource')
      .filter((e) => e.initiatorType !== 'fetch' && e.initiatorType !== 'xmlhttprequest');
    if (urlF) entries = entries.filter((e) => String(e.name || '').toLowerCase().includes(urlF));
    if (failedOnly) entries = entries.filter((e) => typeof e.responseStatus === 'number' && e.responseStatus >= 400);
    resources = entries.slice(-limit).map((e) => {{
      const st = typeof e.responseStatus === 'number' && e.responseStatus > 0 ? ` ${{e.responseStatus}}` : '';
      const ms = Math.round(e.duration || 0);
      const sz = typeof e.transferSize === 'number' && e.transferSize > 0 ? ` ${{e.transferSize}}B` : '';
      return `[${{e.initiatorType || 'resource'}}]${{st}} ${{ms}}ms${{sz}} ${{e.name}}`;
    }});
  }}
  if ({clear}) b.net.length = 0;
  return JSON.stringify({{ ok: true, total, inFlight: b.inFlight, lines, resources }});
}})()"#
    )
}

/// Returns full details (including request/response previews) for one captured network request `#id`.
pub fn network_detail(id: u64) -> String {
    format!(
        r#"{PRELUDE}
(() => {{
  const b = window.__bencode;
  const req = b.net.find((r) => r.id === {id});
  if (!req) return JSON.stringify({{ ok: false, error: 'No captured network request #{id} (call browser_network to list ids).' }});
  return JSON.stringify({{ ok: true, request: req }});
}})()"#
    )
}

/// Inspects one DOM element: selector, box, attributes, outerHTML, and computed CSS styles.
pub fn inspect_element(
    reference: Option<u64>,
    selector: Option<&str>,
    styles: Option<&[String]>,
) -> String {
    let default_styles: &[&str] = &[
        "display",
        "position",
        "visibility",
        "opacity",
        "zIndex",
        "overflow",
        "boxSizing",
        "width",
        "height",
        "margin",
        "padding",
        "border",
        "borderRadius",
        "flexDirection",
        "justifyContent",
        "alignItems",
        "gap",
        "gridTemplateColumns",
        "fontFamily",
        "fontSize",
        "fontWeight",
        "lineHeight",
        "color",
        "backgroundColor",
        "textAlign",
        "pointerEvents",
    ];
    let props_json = match styles {
        Some(list) if !list.is_empty() => serde_json::to_string(list).unwrap_or_else(|_| "[]".into()),
        _ => serde_json::to_string(default_styles).unwrap_or_else(|_| "[]".into()),
    };
    format!(
        "{PRELUDE}\n(() => {{ try {{\n  {}\n  const props = {props_json};\n{}\n}} catch (e) {{ return JSON.stringify({{ ok: false, error: String(e) }}); }} }})()",
        target(reference, selector),
        r#"  const r = el.getBoundingClientRect();
  const cs = getComputedStyle(el);
  const attrs = {};
  for (const a of el.attributes) {
    if (el.localName === 'input' && el.type === 'password' && a.name === 'value') {
      attrs[a.name] = '(redacted)';
    } else {
      attrs[a.name] = a.value.slice(0, 300);
    }
  }
  const computed = {};
  for (const p of props) {
    const cssName = p.includes('-') ? p : p.replace(/[A-Z]/g, (m) => '-' + m.toLowerCase());
    const v = cs.getPropertyValue(cssName);
    if (v) computed[cssName] = v;
  }
  const inViewport = r.bottom > 0 && r.right > 0 && r.top < window.innerHeight && r.left < window.innerWidth;
  let html = el.outerHTML;
  if (el.localName === 'input' && el.type === 'password') {
    html = html.replace(/\svalue="[^"]*"/gi, ' value="(redacted)"');
  }
  if (html.length > 4000) html = html.slice(0, 4000) + '…';
  return JSON.stringify({
    ok: true,
    tag: el.localName,
    described: b.describe(el),
    selector: b.selector(el),
    rect: { x: Math.round(r.left), y: Math.round(r.top), width: Math.round(r.width), height: Math.round(r.height) },
    inViewport,
    visible: cs.display !== 'none' && cs.visibility !== 'hidden' && parseFloat(cs.opacity) > 0 && (r.width > 0 || r.height > 0),
    text: (el.innerText || '').trim().slice(0, 500),
    attributes: attrs,
    styles: computed,
    html,
  });"#
    )
}

/// Reads or mutates `localStorage`, `sessionStorage`, or lists `document.cookie`.
pub fn storage(
    kind: &str,
    action: &str,
    key: Option<&str>,
    value: Option<&str>,
    prefix: Option<&str>,
    include_values: bool,
) -> String {
    let kind_js = serde_json::to_string(kind).unwrap_or_default();
    let action_js = serde_json::to_string(action).unwrap_or_default();
    let key_js = serde_json::to_string(&key.unwrap_or_default()).unwrap_or_default();
    let val_js = serde_json::to_string(&value.unwrap_or_default()).unwrap_or_default();
    let prefix_js = serde_json::to_string(&prefix.unwrap_or_default()).unwrap_or_default();
    format!(
        r#"{PRELUDE}
(() => {{
  try {{
    const kind = {kind_js};
    const action = {action_js};
    const key = {key_js};
    const value = {val_js};
    const prefix = {prefix_js};
    const includeValues = {include_values};
    const redact = (k, v) => {{
      if (!includeValues && /token|secret|password|passwd|auth|session|jwt|cookie|key/i.test(k)) {{
        return `(redacted, ${{String(v).length}} chars — pass includeValues: true to reveal)`;
      }}
      const s = String(v);
      return s.length > 500 ? s.slice(0, 500) + `… (${{s.length}} chars)` : s;
    }};
    if (kind === 'cookies') {{
      if (action !== 'list' && action !== 'get') {{
        return JSON.stringify({{ ok: false, error: 'Cookies support list/get only.' }});
      }}
      const raw = document.cookie ? document.cookie.split('; ').filter(Boolean) : [];
      const entries = raw.map((pair) => {{
        const i = pair.indexOf('=');
        const k = i >= 0 ? pair.slice(0, i) : pair;
        const v = i >= 0 ? pair.slice(i + 1) : '';
        return [k, includeValues ? v : `(${{v.length}} chars)`];
      }}).filter(([k]) => (!key || k === key) && (!prefix || k.startsWith(prefix)));
      return JSON.stringify({{ ok: true, kind, entries }});
    }}
    const store = kind === 'session' ? window.sessionStorage : window.localStorage;
    if (action === 'set') {{
      if (!key) return JSON.stringify({{ ok: false, error: 'storage set requires a key.' }});
      store.setItem(key, value);
      return JSON.stringify({{ ok: true, message: `Set ${{kind}}Storage["${{key}}"].` }});
    }}
    if (action === 'remove') {{
      if (!key) return JSON.stringify({{ ok: false, error: 'storage remove requires a key.' }});
      store.removeItem(key);
      return JSON.stringify({{ ok: true, message: `Removed ${{kind}}Storage["${{key}}"].` }});
    }}
    if (action === 'clear') {{
      const count = store.length;
      store.clear();
      return JSON.stringify({{ ok: true, message: `Cleared ${{count}} keys from ${{kind}}Storage.` }});
    }}
    if (action === 'get' && key) {{
      const v = store.getItem(key);
      if (v === null) return JSON.stringify({{ ok: true, kind, entries: [] }});
      return JSON.stringify({{ ok: true, kind, entries: [[key, redact(key, v)]] }});
    }}
    const entries = [];
    for (let i = 0; i < store.length && entries.length < 100; i++) {{
      const k = store.key(i);
      if (prefix && !k.startsWith(prefix)) continue;
      entries.push([k, redact(k, store.getItem(k))]);
    }}
    return JSON.stringify({{ ok: true, kind, total: store.length, entries }});
  }} catch (e) {{
    return JSON.stringify({{ ok: false, error: String(e) }});
  }}
}})()"#
    )
}

/// Arms, resets, or inspects JS `alert`/`confirm`/`prompt` handling.
pub fn dialog(action: &str, prompt_text: Option<&str>) -> String {
    let action_js = serde_json::to_string(action).unwrap_or_default();
    let prompt_js = match prompt_text {
        Some(text) => serde_json::to_string(text).unwrap_or_default(),
        None => "null".into(),
    };
    format!(
        r#"{PRELUDE}
(() => {{
  const b = window.__bencode;
  const action = {action_js};
  const promptText = {prompt_js};
  if (action === 'accept' || action === 'dismiss') {{
    b.armDialog(action === 'accept', promptText);
    return JSON.stringify({{
      ok: true,
      message: `Armed dialog handler: ${{action}}${{promptText !== null ? ` (promptText="${{promptText}}")` : ''}}.`,
      history: b.dialogs,
    }});
  }}
  if (action === 'reset') {{
    b.resetDialog();
    return JSON.stringify({{ ok: true, message: 'Restored native browser dialogs.', history: b.dialogs }});
  }}
  return JSON.stringify({{
    ok: true,
    armed: b.dialogArmed,
    history: b.dialogs,
  }});
}})()"#
    )
}

/// Checks one wait predicate (`text`, `textGone`, `selector`, `selectorGone`, `url`, `networkIdle`).
pub fn wait_check(
    text: Option<&str>,
    text_gone: Option<&str>,
    selector: Option<&str>,
    selector_gone: Option<&str>,
    url: Option<&str>,
    network_idle: bool,
) -> String {
    let opt = |s: Option<&str>| match s {
        Some(v) => serde_json::to_string(v).unwrap_or_default(),
        None => "null".into(),
    };
    let text_js = opt(text);
    let gone_js = opt(text_gone);
    let sel_js = opt(selector);
    let sel_gone_js = opt(selector_gone);
    let url_js = opt(url);
    format!(
        r#"{PRELUDE}
(() => {{
  const b = window.__bencode;
  const body = document.body ? document.body.innerText : '';
  const wantText = {text_js};
  const goneText = {gone_js};
  const wantSel = {sel_js};
  const goneSel = {sel_gone_js};
  const wantUrl = {url_js};
  const netIdle = {network_idle};
  const visible = (el) => {{
    if (!el) return false;
    const r = el.getBoundingClientRect();
    if (r.width === 0 && r.height === 0) return false;
    const s = getComputedStyle(el);
    return s.display !== 'none' && s.visibility !== 'hidden';
  }};
  let ok = true;
  if (wantText !== null && !body.includes(wantText)) ok = false;
  if (goneText !== null && body.includes(goneText)) ok = false;
  if (wantSel !== null && !visible(b.queryDeep(wantSel))) ok = false;
  if (goneSel !== null && visible(b.queryDeep(goneSel))) ok = false;
  if (wantUrl !== null && !location.href.includes(wantUrl)) ok = false;
  if (netIdle && b.inFlight > 0) ok = false;
  return JSON.stringify({{ ready: ok, inFlight: b.inFlight, url: location.href }});
}})()"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::scripts;
    use serde_json::json;

    #[test]
    fn every_generated_script_parses_in_javascriptcore() {
        let jsc = std::path::Path::new(
            "/System/Library/Frameworks/JavaScriptCore.framework/Versions/A/Helpers/jsc",
        );
        if !jsc.exists() {
            return;
        }
        let samples: [(&str, String); 18] = [
            ("prelude", scripts::PRELUDE.to_string()),
            ("focus", scripts::FOCUS.to_string()),
            ("picker", scripts::picker()),
            ("outline", scripts::outline(Some("main"), 12_000)),
            (
                "click",
                scripts::click(Some(1), None, true, false, &["Shift".into()]),
            ),
            ("type_text", scripts::type_text(None, Some("input"), "hi", true, true)),
            ("press_key", scripts::press_key("Control+Enter", Some(2), None)),
            ("evaluate_inline", scripts::evaluate("1 + 2", 1, true)),
            ("evaluate_eval", scripts::evaluate("const x = 1; x", 2, false)),
            ("eval_result", scripts::eval_result(1)),
            ("console", scripts::console(false, Some("errors"), Some("api"), Some(50))),
            ("hover", hover(Some(3), None)),
            ("select_option", select_option(Some(4), None, &["en".into()])),
            (
                "fill_form",
                fill_form(&json!([{ "ref": 1, "value": "a" }, { "selector": "#ok", "checked": true }]), true),
            ),
            ("scroll", scroll(None, None, Some("down"), Some(300.0), false)),
            ("drag", drag(Some(1), None, Some(2), None)),
            ("element_rect", element_rect(Some(1), None)),
            (
                "network_and_more",
                format!(
                    "{}\n{}\n{}\n{}\n{}\n{}",
                    network_list(Some("/api"), Some("POST"), true, true, 50, false),
                    network_detail(1),
                    inspect_element(Some(1), None, None),
                    storage("local", "list", None, None, Some("app_"), false),
                    dialog("accept", Some("ok")),
                    wait_check(Some("Done"), Some("Loading"), Some("main"), None, Some("/home"), true),
                ),
            ),
        ];

        for (name, js) in samples {
            // `new Function(src)` syntax-checks the script in WebKit's own JS engine without running DOM code.
            let check = format!(
                "new Function({});",
                serde_json::to_string(&js).unwrap()
            );
            let output = std::process::Command::new(jsc)
                .args(["-e", &check])
                .output()
                .expect("run jsc");
            assert!(
                output.status.success(),
                "script `{name}` failed JS syntax check:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
