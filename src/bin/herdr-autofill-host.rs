//! Herdr Autofill's native messaging host. Chromium starts it for each request from the built-in
//! autofill extension: it finds the open site's accounts in the user's vault (the `vault` CLI over
//! a .kdbx), and hands over a password or code only after Touch ID, and only for that site.

#[cfg(target_os = "macos")]
mod host {
    use std::io::{Read, Write};
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::mpsc;
    use std::time::Duration;

    use block::ConcreteBlock;
    use objc::runtime::{BOOL, Object, YES};
    use objc::{class, msg_send, sel, sel_impl};
    use serde_json::{Value, json};

    #[link(name = "LocalAuthentication", kind = "framework")]
    unsafe extern "C" {}

    // Touch ID, or the Mac's password when it isn't available.
    const OWNER_AUTHENTICATION: i64 = 2;
    const CONFIRM_TIME: Duration = Duration::from_secs(120);
    // Second-level labels that are public suffixes, so "example.co.uk" is one site.
    const TWO_PART_SUFFIXES: [&str; 11] =
        ["co.uk", "org.uk", "ac.uk", "gov.uk", "com.au", "net.au", "edu.au", "com.sg", "co.nz", "co.jp", "com.br"];

    struct Account {
        title: String,
        username: String,
        totp: bool,
    }

    pub fn main() {
        let Some(request) = read_message() else { return };
        let reply = answer(&request).unwrap_or_else(|error| json!({ "error": error }));
        write_message(&reply);
    }

    // Chrome's framing: a 4-byte length in native byte order, then that much JSON.
    fn read_message() -> Option<Value> {
        let mut length = [0u8; 4];
        std::io::stdin().read_exact(&mut length).ok()?;
        let mut body = vec![0u8; u32::from_ne_bytes(length) as usize];
        std::io::stdin().read_exact(&mut body).ok()?;
        serde_json::from_slice(&body).ok()
    }

    fn write_message(message: &Value) {
        let body = message.to_string().into_bytes();
        let mut out = std::io::stdout();
        let _ = out.write_all(&(body.len() as u32).to_ne_bytes());
        let _ = out.write_all(&body);
        let _ = out.flush();
    }

    fn answer(request: &Value) -> Result<Value, String> {
        // The extension sets this from the frame that asked, not from anything the page says.
        let url = request["url"].as_str().ok_or("no page")?;
        let site = site_of(&host_of(url).ok_or("not a web page")?);
        let accounts = || -> Result<Vec<Account>, String> {
            Ok(vault(&["list"])?["entries"].as_array().into_iter().flatten()
                .filter(|entry| entry["url"].as_str().and_then(host_of).is_some_and(|host| site_of(&host) == site))
                .map(|entry| Account {
                    title: entry["title"].as_str().unwrap_or_default().to_owned(),
                    username: entry["username"].as_str().unwrap_or_default().to_owned(),
                    totp: entry["hasTotp"].as_bool().unwrap_or(false),
                })
                .collect())
        };
        let chosen = |accounts: Vec<Account>| -> Result<Account, String> {
            let title = request["title"].as_str().ok_or("no account chosen")?;
            accounts.into_iter().find(|account| account.title == title).ok_or_else(|| "that account isn't for this site".into())
        };
        match request["type"].as_str() {
            Some("accounts") => Ok(json!({
                "accounts": accounts()?.iter()
                    .map(|account| json!({ "title": account.title, "username": account.username, "totp": account.totp }))
                    .collect::<Vec<_>>(),
            })),
            Some("fill") => {
                let account = chosen(accounts()?)?;
                confirm(&format!("fill your password for {site}"))?;
                let entry = vault(&["get", &account.title, "--field", "password"])?;
                Ok(json!({ "username": entry["username"], "password": entry["password"] }))
            }
            Some("code") => {
                let account = chosen(accounts()?.into_iter().filter(|account| account.totp).collect())?;
                confirm(&format!("fill a verification code for {site}"))?;
                Ok(json!({ "code": vault(&["totp", &account.title])?["code"] }))
            }
            _ => Err("unknown request".into()),
        }
    }

    fn host_of(url: &str) -> Option<String> {
        let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
        let authority = rest.split(['/', '?', '#']).next()?;
        let host = authority.rsplit('@').next()?.split(':').next()?.to_ascii_lowercase();
        (!host.is_empty()).then_some(host)
    }

    // The registrable part of a host, roughly as browsers group logins: "accounts.example.com" and
    // "www.example.com" are both "example.com". Addresses and single names stay whole.
    fn site_of(host: &str) -> String {
        let labels: Vec<&str> = host.split('.').collect();
        if labels.len() <= 2 || host.parse::<std::net::IpAddr>().is_ok() {
            return host.to_owned();
        }
        let last_two = labels[labels.len() - 2..].join(".");
        let keep = if TWO_PART_SUFFIXES.contains(&last_two.as_str()) { 3 } else { 2 };
        labels[labels.len() - keep..].join(".")
    }

    // The user's vault CLI. HERDR_VAULT names it; HERDR_VAULT_HOME runs it with another HOME, which
    // keeps a test vault apart from the real one (whose path is in ~/.config/opencode/vault.json).
    fn vault(args: &[&str]) -> Result<Value, String> {
        let home = std::env::var("HOME").unwrap_or_default();
        let program = std::env::var("HERDR_VAULT").map(PathBuf::from).unwrap_or_else(|_| {
            let mine = PathBuf::from(&home).join("code/cstack/vault/bin/vault");
            if mine.exists() { mine } else { PathBuf::from("vault") }
        });
        let mut command = Command::new(program);
        command.args(args);
        if let Ok(test_home) = std::env::var("HERDR_VAULT_HOME") {
            command.env("HOME", test_home);
        }
        let output = command.output().map_err(|e| format!("can't run vault: {e}"))?;
        if !output.status.success() {
            let why = String::from_utf8_lossy(&output.stderr);
            return Err(why.lines().next().unwrap_or("vault failed").to_owned());
        }
        serde_json::from_slice(&output.stdout).map_err(|_| "vault answered in a way this doesn't read".into())
    }

    // Touch ID before anything secret leaves the vault, as Safari asks before it fills. macOS shows
    // the reason as "Herdr is trying to <reason>."
    fn confirm(reason: &str) -> Result<(), String> {
        if std::env::var_os("HERDR_AUTOFILL_TEST_CONFIRM").is_some() {
            return Ok(());
        }
        let (sent, answered) = mpsc::channel();
        unsafe {
            let context: *mut Object = msg_send![class!(LAContext), new];
            let text = std::ffi::CString::new(reason).unwrap_or_default();
            let reason: *mut Object = msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()];
            let reply = ConcreteBlock::new(move |ok: BOOL, _error: *mut Object| {
                let _ = sent.send(ok == YES);
            }).copy();
            let _: () = msg_send![context, evaluatePolicy: OWNER_AUTHENTICATION localizedReason: reason reply: &*reply];
        }
        match answered.recv_timeout(CONFIRM_TIME) {
            Ok(true) => Ok(()),
            _ => Err("not confirmed".into()),
        }
    }
}

#[cfg(target_os = "macos")]
fn main() {
    host::main();
}

#[cfg(not(target_os = "macos"))]
fn main() {}
