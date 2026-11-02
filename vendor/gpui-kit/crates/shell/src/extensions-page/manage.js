// Herdr's extensions page: what Chrome's own page does, inside a browser pane.
const list = document.getElementById("list");

function make(tag, props = {}, ...children) {
  const element = Object.assign(document.createElement(tag), props);
  element.append(...children);
  return element;
}

function row(extension) {
  // Loaded by Herdr itself from a folder, so it comes back on the next start whatever is done here.
  const builtIn = extension.installType === "development";
  const icon = make("img", { src: extension.icons?.at(-1)?.url ?? "", alt: "" });
  icon.onerror = () => { icon.style.visibility = "hidden"; };
  const about = make("div", { className: "about" },
    make("div", { className: "name" }, extension.name,
      make("span", { className: "version" }, extension.version),
      ...(builtIn ? [make("span", { className: "kind" }, "· built in")] : [])),
    make("div", { className: "description", title: extension.description }, extension.description));
  const buttons = [];
  if (extension.optionsUrl) {
    // Chrome won't let one extension's page navigate into another's, but Herdr opens what a page
    // asks to open in a new window in this same pane, and that's allowed.
    buttons.push(make("button", { onclick: () => window.open(extension.optionsUrl) }, "Options"));
  }
  if (!builtIn) {
    buttons.push(make("button", {
      onclick: async () => {
        if (confirm(`Remove ${extension.name}?`)) await chrome.management.uninstall(extension.id, { showConfirmDialog: false });
      },
    }, "Remove"));
  }
  const toggle = make("button", {
    className: `switch${extension.enabled ? " on" : ""}`,
    title: extension.enabled ? "Turn off" : "Turn on",
    onclick: () => chrome.management.setEnabled(extension.id, !extension.enabled),
  });
  return make("li", { className: extension.enabled ? "" : "off" }, icon, about, ...buttons, toggle);
}

async function render() {
  const extensions = (await chrome.management.getAll())
    .filter((extension) => extension.id !== chrome.runtime.id && extension.type === "extension")
    .sort((a, b) => a.name.localeCompare(b.name));
  list.replaceChildren(...extensions.map(row));
  document.getElementById("empty").hidden = extensions.length > 0;
}

for (const event of ["onInstalled", "onUninstalled", "onEnabled", "onDisabled"]) chrome.management[event].addListener(render);
render();
