// Herdr Autofill in the page: a menu of the vault's accounts under a login field (one account fills
// straight away), and a verification code for a code field. Filling asks Touch ID first.
(() => {
  if (window.__herdrAutofill) return;
  window.__herdrAutofill = true;

  // Right after the browser starts, the extension's worker may not be up yet; ask again shortly.
  // A lookup that hangs gives up rather than leaving the page waiting; a fill waits for Touch ID.
  const LOOKUP_WAIT = 5_000;
  const TOUCH_ID_WAIT = 130_000;
  const ASKS_FOR_TOUCH_ID = new Set(["fill", "code", "card", "passkey"]);
  async function ask(message) {
    const wait = ASKS_FOR_TOUCH_ID.has(message.type) ? TOUCH_ID_WAIT : LOOKUP_WAIT;
    for (let attempt = 0; attempt < 3; attempt++) {
      try {
        const answer = await Promise.race([chrome.runtime.sendMessage(message), new Promise((ok) => setTimeout(() => ok(undefined), wait))]);
        if (answer !== undefined) return answer;
      } catch {}
      await new Promise((ok) => setTimeout(ok, 400));
    }
    return {};
  }
  const MENU_GAP = 4;
  const CODE_HINT = /(otp|totp|2fa|one.?time|verification|security.?code|auth.?code|passcode)/;
  const USER_HINT = /(user|email|e-mail|login|account|identifier)/;
  // Each kind (a login, a code, an address, a card) fills by itself once per form a page load, so a
  // cancelled Touch ID doesn't come back on every focus, and an address doesn't stop the card.
  const offered = new WeakMap();
  function firstOffer(scope, what) {
    const kinds = offered.get(scope) ?? new Set();
    if (kinds.has(what)) return false;
    offered.set(scope, kinds.add(what));
    return true;
  }
  let menu = null;
  // Focus moved by filling isn't a person asking for the menu.
  let filling = false;

  const visible = (el) => el.getClientRects().length > 0 && getComputedStyle(el).visibility !== "hidden";
  const hint = (el) => `${el.name} ${el.id} ${el.placeholder} ${el.getAttribute("aria-label") ?? ""}`.toLowerCase();
  const passwords = (scope) => [...scope.querySelectorAll("input[type=password]")]
    .filter((el) => visible(el) && !el.disabled && !(el.autocomplete ?? "").includes("new-password"));

  // Checkout fields by their autocomplete token, or failing that by their name, id or label.
  const ADDRESS_TOKENS = new Set(["name", "given-name", "family-name", "organization", "street-address", "address-line1",
    "address-line2", "address-level2", "address-level1", "postal-code", "country", "country-name"]);
  const TOKEN_HINTS = [
    ["cc-number", /(card.?number|cardnum|cc.?num|card.?no\b)/],
    ["cc-csc", /(cvc|cvv|csc|security.?code|card.?code)/],
    ["cc-exp-month", /(exp\w*.?month|cc.?month|card.?month)/],
    ["cc-exp-year", /(exp\w*.?year|cc.?year|card.?year)/],
    ["cc-exp", /(expir|exp.?date|mm.?yy)/],
    ["cc-name", /(name.?on.?card|card.?holder|cc.?name)/],
    ["given-name", /(first.?name|given.?name|fname)/],
    ["family-name", /(last.?name|family.?name|surname|lname)/],
    ["address-line2", /(address.?2|address.?line.?2|apartment|suite|\bapt\b|\bunit\b)/],
    ["address-line1", /(address.?1|address.?line.?1|street|address)/],
    ["address-level2", /(city|town|suburb)/],
    ["address-level1", /(state|province|region|county)/],
    ["postal-code", /(zip|postal|post.?code)/],
    ["country", /country/],
    ["organization", /(company|organi[sz]ation)/],
    ["tel", /(phone|mobile|\btel\b)/],
    ["email", /e.?mail/],
    ["name", /(full.?name|^name$|\bname\b)/],
  ];

  function token(el) {
    const tokens = (el.autocomplete ?? "").toLowerCase().split(/\s+/).filter((part) => part && !/^(section-|shipping|billing|home|work|on|off)/.test(part));
    if (tokens.length) return tokens[tokens.length - 1];
    const label = el.labels?.[0]?.textContent ?? "";
    const text = `${hint(el)} ${label.toLowerCase()}`;
    return TOKEN_HINTS.find(([, pattern]) => pattern.test(text))?.[0] ?? null;
  }

  function kind(el) {
    if (!(el instanceof HTMLInputElement) || el.disabled || el.readOnly) return null;
    const auto = (el.autocomplete ?? "").toLowerCase();
    if (auto.includes("one-time-code") || (el.type !== "password" && CODE_HINT.test(hint(el)))) return "code";
    if (el.type !== "password") {
      const which = token(el);
      if (which?.startsWith("cc-")) return "card";
      if (ADDRESS_TOKENS.has(which) && !passwords(el.form ?? document).length) return "address";
    }
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

  // A code field: the site's vault codes, and a recent one from Messages. One that's plainly this
  // site's fills straight away; a Messages code that might be another site's waits for a click.
  async function onCodeField(field, accounts) {
    const scope = field.form ?? document;
    const fromMessages = await ask({ type: "messages" });
    if (document.activeElement !== field) return;
    const totp = accounts.filter((account) => account.totp);
    if (fromMessages?.code && fromMessages.forSite && firstOffer(scope, "code")) return fillCode(field, fromMessages.code);
    if (!fromMessages?.code && totp.length === 1 && firstOffer(scope, "code")) return codeFrom(field, totp[0]);
    const rows = totp.map((account) => ({ label: account.username || account.title, pick: () => codeFrom(field, account) }));
    if (fromMessages?.code) rows.unshift({ label: `${fromMessages.code} · From Messages`, pick: () => { hide(); fillCode(field, fromMessages.code); } });
    if (rows.length) show(field, rows);
  }

  // What a checkout field gets from an address entry's fields.
  function addressValue(which, fields) {
    const [given = "", ...family] = (fields.name ?? "").split(" ");
    return {
      "name": fields.name, "given-name": given, "family-name": family.join(" "), "organization": fields.organization,
      "street-address": [fields.street, fields.street2].filter(Boolean).join("\n"), "address-line1": fields.street,
      "address-line2": fields.street2, "address-level2": fields.city, "address-level1": fields.state,
      "postal-code": fields.postcode, "country": fields.country, "country-name": fields.country, "tel": fields.phone, "email": fields.email,
    }[which];
  }

  // A card's expiry, stored MM/YY, written as the field wants it.
  function cardValue(which, fields, el) {
    const [month = "", year = ""] = (fields.expiry ?? "").split("/").map((part) => part.trim());
    const fullYear = year.length === 2 ? `20${year}` : year;
    const wantsFull = el.maxLength >= 4 && el.maxLength < 6 ? false : el.maxLength === 4 || /yyyy/i.test(el.placeholder);
    return {
      "cc-name": fields.cardholder, "cc-number": fields.number, "cc-csc": fields.cvc,
      "cc-exp": el.maxLength === 7 || /yyyy/i.test(el.placeholder) ? `${month}/${fullYear}` : `${month}/${year.slice(-2)}`,
      "cc-exp-month": month, "cc-exp-year": el instanceof HTMLSelectElement || wantsFull ? fullYear : year.slice(-2),
    }[which];
  }

  // A choice in a select: by value or by shown text, so "IL" or "Illinois", "08" or "8" or "August".
  function choose(select, value) {
    const want = String(value).toLowerCase();
    const option = [...select.options].find((o) => o.value.toLowerCase() === want || o.text.trim().toLowerCase() === want)
      ?? [...select.options].find((o) => /^\d+$/.test(want) && Number(o.value) === Number(want))
      ?? [...select.options].find((o) => o.text.trim().toLowerCase().startsWith(want))
      ?? [...select.options].find((o) => /^\d{2}$/.test(want) && (o.value.endsWith(want) || o.text.trim().endsWith(want)));
    if (!option) return;
    select.value = option.value;
    select.dispatchEvent(new Event("input", { bubbles: true }));
    select.dispatchEvent(new Event("change", { bubbles: true }));
  }

  function fillForm(field, valueFor) {
    const scope = field.form ?? document;
    for (const el of scope.querySelectorAll("input, select, textarea")) {
      if (!visible(el) || el.disabled || el.readOnly) continue;
      const value = valueFor(token(el), el);
      if (!value) continue;
      if (el instanceof HTMLSelectElement) choose(el, value);
      else if (!el.value) setValue(el, value);
    }
  }

  async function onAddressField(field) {
    const { addresses = [] } = await ask({ type: "addresses" });
    if (document.activeElement !== field || !addresses.length) return;
    const scope = field.form ?? document;
    const pick = (address) => { hide(); fillForm(field, (which) => addressValue(which, address.fields)); };
    if (addresses.length === 1 && firstOffer(scope, "address")) return pick(addresses[0]);
    show(field, addresses.map((address) => ({ label: address.title, pick: () => pick(address) })));
  }

  async function onCardField(field) {
    const { cards = [] } = await ask({ type: "cards" });
    if (document.activeElement !== field || !cards.length) return;
    const scope = field.form ?? document;
    const pick = async (card) => {
      hide();
      const got = await ask({ type: "card", title: card.title });
      if (got?.fields) fillForm(field, (which, el) => cardValue(which, got.fields, el));
    };
    if (cards.length === 1 && firstOffer(scope, "card")) return pick(cards[0]);
    show(field, cards.map((card) => ({ label: card.title, pick: () => pick(card) })));
  }

  async function onFocus(event) {
    const field = event.target;
    const what = filling ? null : kind(field);
    if (!what) return;
    if (what === "address") return onAddressField(field);
    if (what === "card") return onCardField(field);
    const { accounts = [] } = await ask({ type: "accounts" });
    if (document.activeElement !== field) return;
    if (what === "code") return onCodeField(field, accounts);
    const scope = field.form ?? document;
    const choices = accounts;
    if (!choices.length) return;
    const pick = (account) => fillLogin(field, account);
    if (choices.length === 1 && firstOffer(scope, "login")) return pick(choices[0]);
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

  // A site's passkey request, relayed from the page's world (webauthn.js); one at a time.
  let passkeyBusy = false;
  document.addEventListener("herdr-passkey", async (event) => {
    let request;
    try { request = JSON.parse(event.detail); } catch { return; }
    const answer = passkeyBusy ? {} : (passkeyBusy = true, await ask({ type: "passkey", mode: request.mode === "create" ? "create" : "get", rpId: String(request.rpId ?? "") }));
    if (!passkeyBusy) return;
    passkeyBusy = false;
    document.dispatchEvent(new CustomEvent("herdr-passkey-ready", { detail: JSON.stringify({ id: request.id, ...answer }) }));
  });

  document.addEventListener("focusin", onFocus, true);
  // A field focused before this arrived (autofocus) gets its turn now.
  if (document.activeElement instanceof HTMLInputElement) onFocus({ target: document.activeElement });
  window.addEventListener("scroll", hide, true);
  window.addEventListener("resize", hide);
})();
