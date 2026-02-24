# CoveMail Threat Model & Abuse-Case Checklist

## Core Principles
- **Local-First:** All mail data is stored locally. No cloud middleman or proxies.
- **Zero Telemetry:** The application phones home to no one. Only communicates with the email provider chosen by the user.
- **Secure by Default:** Secrets are stored in the OS Keychain, databases can be encrypted using SQLCipher. 

## Threat Surface
1. **Malicious Email Content**
   - *Threat:* XSS, tracking pixels, or exploit execution via malicious HTML bodies.
   - *Mitigation:* Uses `ammonia` to aggressively sanitize and neutralize HTML content before passing it to `egui` labels.
2. **Malicious Attachments**
   - *Threat:* Executable attachments designed to run automatically or easily trick users.
   - *Mitigation:* We strictly block opening or saving dangerous extensions natively (e.g. `.exe`, `.bat`, `.sh`, `.app`, `.scr`).
3. **Secret Logging & Leakage**
   - *Threat:* OAuth tokens or passwords accidentally logged to stdout or crash reporting.
   - *Mitigation:* Grep checks assert no trace of token/key logging in our codebase. Keychain handles safe key transit.
4. **Local Network Sniffing**
   - *Threat:* Insecure connections intercepted via MITM attacks.
   - *Mitigation:* Required TLS enforcement for IMAP, SMTP, and CalDAV communication.
5. **Database Extraction**
   - *Threat:* Another local application tries to read `.sqlite` containing email messages.
   - *Mitigation:* Support for encrypted SQLite (SQLCipher) via a keychain-held database key.

## Abuse-Case Checklist
- [x] Does rendering an email with embedded `<script>` tags execute JS? (No, `ammonia` blocks tags)
- [x] Does downloading `invoice.exe.zip` allow blind execution? (No, requires manual user extraction)
- [x] Does downloading `payload.sh` allow saving? (No, explicitly blocked by UI attachment extension logic)
- [x] Can the user inadvertently send their API key in error logs? (No, logging of credentials completely stripped/omitted)
- [x] Does connecting to an untrusted IMAP server allow downgrading to plaintext auth? (No, TLS config strictly bound)
- [x] Does the UI renderer (`egui`) support WebGL injection attacks from HTML? (No, strictly maps to native rect calls without web views)
