// Passes a page's requests to Herdr's autofill host, naming the page by the frame Chrome says asked,
// never by anything the page itself claims.
const HOST = "dev.herdr.autofill";
const ACCOUNTS_KEEP = 60_000;
const accounts = new Map();

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
    return { error: "unknown request" };
  })().then(reply, (error) => reply({ error: String(error) }));
  return true;
});
