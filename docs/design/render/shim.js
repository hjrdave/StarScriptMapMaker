// Stand-in for the design tool's support.js: expands sc-for / sc-if / {{expr}} from renderVals().
window.DCLogic = class {};
function evalIn(expr, scope) {
  const keys = Object.keys(scope);
  try { return new Function(...keys, 'return (' + expr + ')')(...keys.map(k => scope[k])); }
  catch (e) { return ''; }
}
function render(str, scope) {
  str = str.replace(/<sc-for\s+list="\{\{([^}]*)\}\}"\s+as="(\w+)"[^>]*>([\s\S]*?)<\/sc-for>/g,
    (_, list, as, inner) => (evalIn(list, scope) || []).map(item => render(inner, { ...scope, [as]: item })).join(''));
  str = str.replace(/<sc-if\s+value="\{\{([^}]*)\}\}"[^>]*>([\s\S]*?)<\/sc-if>/g,
    (_, cond, inner) => evalIn(cond, scope) ? inner : '');
  return str.replace(/\{\{([^}]*)\}\}/g, (_, e) => { const v = evalIn(e, scope); return v == null ? '' : String(v); });
}
document.addEventListener('DOMContentLoaded', () => {
  const s = document.querySelector('script[type="text/x-dc"]');
  if (!s) return;
  const Component = new Function(s.textContent + '; return Component;')();
  const vals = new Component().renderVals() || {};
  s.remove();
  document.body.innerHTML = render(document.body.innerHTML, vals);
});
// The mockups' dock-button rule `.tool` also matches the Properties tool section `.sec.tool`
// and squashes it to 32×30 px; Label and NodeEdit also have a stray empty flex:1 spacer at the
// top of the window that pushes it down. Undo both for the screenshots only.
document.addEventListener('DOMContentLoaded', () => {
  const st = document.createElement('style');
  st.textContent = '.win > div[style="flex:1"]:empty{display:none} .sec.tool{width:auto;height:auto;display:block;border-radius:0;padding:6px 12px 10px;color:inherit;justify-content:initial;align-items:initial}';
  document.head.appendChild(st);
});
