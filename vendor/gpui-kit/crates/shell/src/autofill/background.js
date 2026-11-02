// Passes a page's requests to Herdr's autofill host, naming the page by the frame Chrome says asked,
// never by anything the page itself claims.
const HOST = "dev.herdr.autofill";
const ACCOUNTS_KEEP = 60_000;
const accounts = new Map();
const typed = new Map();
const TYPED_KEEP = 60_000;
// Chromium stops an idle extension worker after 30 seconds and doesn't always manage to start it
// again, and a page asking a stopped worker waits for good. A small call every 20 seconds keeps it up.
const KEEP_AWAKE = 20_000;
setInterval(() => chrome.runtime.getPlatformInfo(() => {}), KEEP_AWAKE);

chrome.runtime.onMessage.addListener((message, sender, reply) => {
  const url = sender.url;
  if (!url || !/^https?:/.test(url)) return false;
  const ask = (request) => chrome.runtime.sendNativeMessage(HOST, { ...request, url });
  (async () => {
    if (message.type === "accounts") {
      // Names only, so they're kept a minute; a password is fetched, after Touch ID, each time.
      const site = new URL(url).host;
      const kept = accounts.get(site);
      if (kept && Date.now() - kept.at < ACCOUNTS_KEEP) return kept.value;
      const value = await ask({ type: "accounts" });
      if (!value.error) accounts.set(site, { at: Date.now(), value });
      return value;
    }
    if (message.type === "fill" || message.type === "code") return ask({ type: message.type, title: message.title });
    // A typed login waits here (never in the page) for the page's offer to save it.
    if (message.type === "typed") {
      const known = await ask({ type: "known", username: message.username, password: message.password });
      if (known.same || known.error) return {};
      typed.set(new URL(url).host, { username: message.username, password: message.password, update: !!known.update, at: Date.now() });
      return { pending: true };
    }
    if (message.type === "pending") {
      const login = typed.get(new URL(url).host);
      if (!login || Date.now() - login.at > TYPED_KEEP) return {};
      return { username: login.username, update: login.update };
    }
    if (message.type === "save") {
      const site = new URL(url).host, login = typed.get(site);
      typed.delete(site);
      if (!login) return { error: "nothing to save" };
      accounts.delete(site);
      return ask({ type: "save", username: login.username, password: login.password });
    }
    if (message.type === "dismiss") {
      typed.delete(new URL(url).host);
      return {};
    }
    if (message.type === "messages" || message.type === "addresses" || message.type === "cards") return ask({ type: message.type });
    if (message.type === "card") return ask({ type: "card", title: message.title });
    if (message.type === "passkey") return ask({ type: "passkey", mode: message.mode, rpId: message.rpId });
    return { error: "unknown request" };
  })().then(reply, (error) => reply({ error: String(error) }));
  return true;
});
