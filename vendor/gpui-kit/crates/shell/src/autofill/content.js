// Herdr Autofill in the page: a menu of the vault's accounts under a login field (one account fills
// straight away), and a verification code for a code field. Filling asks Touch ID first.
(() => {
  if (window.__herdrAutofill) return;
  window.__herdrAutofill = true;

  const ask = (message) => chrome.runtime.sendMessage(message).catch(() => ({}));
  const MENU_GAP = 4;
  const CODE_HINT = /(otp|totp|2fa|one.?time|verification|security.?code|auth.?code|passcode)/;
  const USER_HINT = /(user|email|e-mail|login|account|identifier)/;
  // A form is offered once a page load: one cancelled Touch ID doesn't come back on every focus.
  const offered = new WeakSet();
  let menu = null;
  // Focus moved by filling isn't a person asking for the menu.
  let filling = false;

  const visible = (el) => el.getClientRects().length > 0 && getComputedStyle(el).visibility !== "hidden";
  const hint = (el) => `${el.name} ${el.id} ${el.placeholder} ${el.getAttribute("aria-label") ?? ""}`.toLowerCase();
  const passwords = (scope) => [...scope.querySelectorAll("input[type=password]")]
    .filter((el) => visible(el) && !el.disabled && !(el.autocomplete ?? "").includes("new-password"));

  function kind(el) {
    if (!(el instanceof HTMLInputElement) || el.disabled || el.readOnly) return null;
    const auto = (el.autocomplete ?? "").toLowerCase();
    if (auto.includes("one-time-code") || (el.type !== "password" && CODE_HINT.test(hint(el)))) return "code";
    if (el.type === "password") return auto.includes("new-password") ? null : "password";
    if (!["text", "email", "tel", ""].includes(el.type)) return null;
    if (auto.includes("username") || el.type === "email" || auto === "email") return "username";
    if (USER_HINT.test(hint(el)) && passwords(el.form ?? document).length) return "username";
    return null;
  }

  // Through the page's own value setter, so frameworks that watch input events notice.
  function setValue(el, value) {
    filling = true;
    el.focus();
    filling = false;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
  }

  function usernameFor(scope, field) {
    if (kind(field) === "username") return field;
    const candidates = [...scope.querySelectorAll("input")].filter((el) => visible(el) && kind(el) === "username");
    return candidates[0] ?? null;
  }

  async function fillLogin(field, account) {
    hide();
    const got = await ask({ type: "fill", title: account.title });
    if (got?.error || !got) return;
    const scope = field.form ?? document;
    const user = usernameFor(scope, field);
    const pass = kind(field) === "password" ? field : passwords(scope)[0];
    if (user && got.username) setValue(user, got.username);
    if (pass && got.password) setValue(pass, got.password);
  }

  // A code split across one-character boxes goes in box by box.
  function fillCode(field, code) {
    const boxes = field.maxLength === 1
      ? [...(field.form ?? field.parentElement?.parentElement ?? document).querySelectorAll("input")].filter((el) => el.maxLength === 1 && visible(el))
      : [field];
    if (boxes.length > 1) [...code].forEach((digit, i) => boxes[i] && setValue(boxes[i], digit));
    else setValue(field, code);
  }

  async function codeFrom(field, account) {
    hide();
    const got = await ask({ type: "code", title: account.title });
    if (got?.code) fillCode(field, got.code);
  }

  async function onFocus(event) {
    const field = event.target;
    const what = filling ? null : kind(field);
    if (!what) return;
    const { accounts = [] } = await ask({ type: "accounts" });
    if (document.activeElement !== field) return;
    const scope = field.form ?? document;
    const choices = what === "code" ? accounts.filter((account) => account.totp) : accounts;
    if (!choices.length) return;
    const pick = what === "code" ? (account) => codeFrom(field, account) : (account) => fillLogin(field, account);
    if (choices.length === 1 && !offered.has(scope)) {
      offered.add(scope);
      return pick(choices[0]);
    }
    show(field, choices.map((account) => ({ label: account.username || account.title, pick: () => pick(account) })));
  }

  const STYLE = `
    :host { all: initial; }
    .menu { position: fixed; z-index: 2147483647; box-sizing: border-box; padding: 3px; border-radius: 7px;
      background: #ffffff; color: #1f2328; border: 1px solid #d8dbe0; box-shadow: 0 8px 24px rgba(0,0,0,.18);
      font: 13px/1.35 -apple-system, system-ui, sans-serif; }
    .row { height: 26px; line-height: 26px; padding: 0 8px; border-radius: 5px; cursor: default;
      white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .row.at { background: #3b82f6; color: #ffffff; }
    @media (prefers-color-scheme: dark) {
      .menu { background: #2b2f36; color: #dcdfe4; border-color: #3d424b; box-shadow: 0 8px 24px rgba(0,0,0,.45); }
      .row.at { background: #2f6fde; }
    }`;

  function show(field, rows) {
    hide();
    const host = document.createElement("herdr-autofill");
    host.dataset.rows = String(rows.length);
    const root = host.attachShadow({ mode: "closed" });
    const box = document.createElement("div");
    box.className = "menu";
    const style = document.createElement("style");
    style.textContent = STYLE;
    root.append(style, box);
    const rect = field.getBoundingClientRect();
    // As wide as the field, one short line per account.
    Object.assign(box.style, { top: `${rect.bottom + MENU_GAP}px`, left: `${rect.left}px`, width: `${rect.width}px` });
    const items = rows.map((row, i) => {
      const item = document.createElement("div");
      item.className = "row";
      item.textContent = row.label;
      // Pressing keeps the field focused, so the menu isn't closed before the click lands.
      item.addEventListener("mousedown", (e) => e.preventDefault());
      item.addEventListener("click", () => row.pick());
      item.addEventListener("mouseenter", () => select(i));
      box.append(item);
      return item;
    });
    document.documentElement.append(host);
    menu = { host, field, rows, items, at: -1 };
    field.addEventListener("keydown", onKey, true);
    field.addEventListener("blur", hide, { once: true });
  }

  function select(i) {
    if (!menu) return;
    menu.at = i;
    menu.items.forEach((item, j) => item.classList.toggle("at", j === i));
  }

  function onKey(e) {
    if (!menu) return;
    const count = menu.rows.length;
    if (e.key === "ArrowDown") select((menu.at + 1) % count);
    else if (e.key === "ArrowUp") select((menu.at - 1 + count) % count);
    else if (e.key === "Enter" && menu.at >= 0) menu.rows[menu.at].pick();
    else if (e.key === "Escape") hide();
    else return;
    e.preventDefault();
    e.stopPropagation();
  }

  function hide() {
    if (!menu) return;
    menu.field.removeEventListener("keydown", onKey, true);
    menu.host.remove();
    menu = null;
  }

  document.addEventListener("focusin", onFocus, true);
  // A field focused before this arrived (autofocus) gets its turn now.
  if (document.activeElement instanceof HTMLInputElement) onFocus({ target: document.activeElement });
  window.addEventListener("scroll", hide, true);
  window.addEventListener("resize", hide);
})();
