// Plotica desktop (Tauri v2) backend.
//
// Implements Google sign-in for a desktop webview WITHOUT Firebase's hosted
// popup/redirect handler (which cannot run inside the Tauri webview origin).
// Flow:
//   1. Generate a PKCE verifier/challenge + a random `state`.
//   2. Bind a one-shot loopback HTTP server on 127.0.0.1:<random free port>.
//   3. Open the user's SYSTEM browser at Google's consent screen, with
//      redirect_uri = http://127.0.0.1:<port>.
//   4. Capture the ?code=... redirect on the loopback server.
//   5. Exchange the code (+ PKCE verifier) for an id_token / access_token.
//   6. Return the tokens to the frontend, which calls
//      firebase.auth().signInWithCredential(...).
//
// Loopback redirects are accepted by Google "Desktop app" OAuth clients on ANY
// port, so no fixed redirect URI / domain needs to be registered.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

#[derive(Serialize)]
pub struct Tokens {
    pub id_token: String,
    pub access_token: String,
}

fn b64url(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

fn random_b64url(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    rand::thread_rng().fill_bytes(&mut bytes);
    b64url(&bytes)
}

#[tauri::command]
async fn google_login(
    client_id: String,
    client_secret: Option<String>,
    scopes: Option<String>,
) -> Result<Tokens, String> {
    tauri::async_runtime::spawn_blocking(move || do_login(client_id, client_secret, scopes))
        .await
        .map_err(|e| e.to_string())?
}

fn do_login(
    client_id: String,
    client_secret: Option<String>,
    scopes: Option<String>,
) -> Result<Tokens, String> {
    let scope = scopes.unwrap_or_else(|| "openid email profile".to_string());

    // PKCE
    let verifier = random_b64url(32);
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    let state = random_b64url(16);

    // Loopback server on a random free port.
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("loopback bind: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| e.to_string())?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}");

    // Build Google's consent URL.
    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={cid}&redirect_uri={ru}&response_type=code&scope={sc}&code_challenge={cc}&code_challenge_method=S256&state={st}&access_type=offline&prompt=select_account",
        cid = urlencoding::encode(&client_id),
        ru = urlencoding::encode(&redirect_uri),
        sc = urlencoding::encode(&scope),
        cc = challenge,
        st = state,
    );

    // Open the system browser.
    webbrowser::open(&auth_url).map_err(|e| format!("tarayici acilamadi: {e}"))?;

    // Wait for the redirect carrying the authorization code.
    let (code, got_state, err) = wait_for_redirect(&listener)?;
    if let Some(err) = err {
        return Err(format!("Google reddetti: {err}"));
    }
    if got_state != state {
        return Err("state uyusmuyor (guvenlik)".to_string());
    }
    if code.is_empty() {
        return Err("yetkilendirme kodu alinamadi".to_string());
    }

    // Exchange the code for tokens.
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let mut form: Vec<(&str, &str)> = vec![
        ("client_id", client_id.as_str()),
        ("code", code.as_str()),
        ("code_verifier", verifier.as_str()),
        ("grant_type", "authorization_code"),
        ("redirect_uri", redirect_uri.as_str()),
    ];
    if let Some(ref secret) = client_secret {
        if !secret.is_empty() {
            form.push(("client_secret", secret.as_str()));
        }
    }

    let resp: serde_json::Value = client
        .post("https://oauth2.googleapis.com/token")
        .form(&form)
        .send()
        .map_err(|e| format!("token istegi: {e}"))?
        .json()
        .map_err(|e| format!("token yaniti: {e}"))?;

    if let Some(err) = resp.get("error") {
        let desc = resp
            .get("error_description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        return Err(format!("token hatasi: {err} {desc}"));
    }

    let id_token = resp
        .get("id_token")
        .and_then(|v| v.as_str())
        .ok_or("yanitta id_token yok")?
        .to_string();
    let access_token = resp
        .get("access_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Ok(Tokens {
        id_token,
        access_token,
    })
}

/// Blocks until the OAuth provider hits the loopback URL, then returns
/// (code, state, error). Responds to the browser with a friendly page.
fn wait_for_redirect(listener: &TcpListener) -> Result<(String, String, Option<String>), String> {
    for stream in listener.incoming() {
        let mut stream = stream.map_err(|e| e.to_string())?;
        let mut buf = [0u8; 8192];
        let n = stream.read(&mut buf).map_err(|e| e.to_string())?;
        let req = String::from_utf8_lossy(&buf[..n]);

        // First request line: "GET /?code=...&state=... HTTP/1.1"
        let first = req.lines().next().unwrap_or("");
        let path = first.split_whitespace().nth(1).unwrap_or("");
        let query = path.splitn(2, '?').nth(1).unwrap_or("");

        let mut code = String::new();
        let mut state = String::new();
        let mut error: Option<String> = None;
        for pair in query.split('&') {
            let mut kv = pair.splitn(2, '=');
            let key = kv.next().unwrap_or("");
            let raw = kv.next().unwrap_or("");
            let val = urlencoding::decode(raw)
                .map(|c| c.into_owned())
                .unwrap_or_default();
            match key {
                "code" => code = val,
                "state" => state = val,
                "error" => error = Some(val),
                _ => {}
            }
        }

        // Respond with a small page so the browser tab is friendly.
        let body = "<!doctype html><html lang=\"tr\"><head><meta charset=\"utf-8\"><title>Plotica</title></head>\
<body style=\"font-family:system-ui,sans-serif;background:#0e0e0f;color:#f0ede6;text-align:center;padding-top:90px;margin:0\">\
<div style=\"font-family:Georgia,serif;font-size:34px;font-weight:900;color:#e8c97a\">Plotica</div>\
<p style=\"color:#9a9690;margin-top:14px\">Giris tamamlandi. Bu sekmeyi kapatip uygulamaya donebilirsiniz.</p>\
<script>setTimeout(function(){window.close();},800);</script></body></html>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.as_bytes().len(),
            body
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();

        // Ignore favicon / other noise; only return once we actually have a result.
        if !code.is_empty() || error.is_some() {
            return Ok((code, state, error));
        }
    }
    Err("loopback dinleyici kapandi".to_string())
}

/* ═══════════════ DIŞA AKTARMA ═══════════════
   Tauri webview'i <a download> ve window.open+print'i engeller, bu yüzden
   dosya kaydetme ve yazdırma Rust tarafından yapılır. */

use tauri_plugin_dialog::DialogExt;

/// Windows'ta yasak olan dosya adı karakterlerini temizler.
fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if "\\/:*?\"<>|\r\n\t".contains(c) { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').to_string();
    if trimmed.is_empty() { "Adsiz".to_string() } else { trimmed }
}

/// "Farklı kaydet" penceresi açar ve metni UTF-8 olarak yazar.
/// Dönüş: Some(yol) = kaydedildi, None = kullanıcı iptal etti.
#[tauri::command]
fn save_text_file(
    app: tauri::AppHandle,
    suggested_name: String,
    contents: String,
    extension: Option<String>,
    bom: Option<bool>,
) -> Result<Option<String>, String> {
    let ext = extension
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| {
            std::path::Path::new(&suggested_name)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("txt")
                .to_string()
        })
        .to_lowercase();

    let label = match ext.as_str() {
        "txt" => "Metin dosyasi (*.txt)",
        "csv" => "CSV tablosu (*.csv)",
        "fdx" => "Final Draft senaryo (*.fdx)",
        "html" => "HTML dosyasi (*.html)",
        _ => "Dosya",
    };

    let picked = app
        .dialog()
        .file()
        .set_title("Farkli Kaydet")
        .set_file_name(&sanitize_file_name(&suggested_name))
        .add_filter(label, &[ext.as_str()])
        .add_filter("Tum dosyalar", &["*"])
        .blocking_save_file();

    let Some(fp) = picked else { return Ok(None) };

    let mut path = fp.into_path().map_err(|e| e.to_string())?;
    if path.extension().is_none() {
        path.set_extension(&ext);
    }

    // Windows uygulamaları için CRLF satır sonu
    let text = contents.replace("\r\n", "\n").replace('\n', "\r\n");

    let mut bytes: Vec<u8> = Vec::with_capacity(text.len() + 3);
    if bom.unwrap_or(false) {
        bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]); // Excel'in CSV'yi UTF-8 okuması için
    }
    bytes.extend_from_slice(text.as_bytes());

    std::fs::write(&path, &bytes).map_err(|e| format!("dosya yazilamadi: {e}"))?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// Yolu file:// URL'ine çevirir (Türkçe karakter + boşluk güvenli).
fn file_url(p: &std::path::Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = s.trim_start_matches('/');
    let mut out = String::from("file:///");
    for (i, seg) in s.split('/').enumerate() {
        if i > 0 {
            out.push('/');
        }
        if i == 0 && seg.len() == 2 && seg.ends_with(':') {
            out.push_str(seg); // "C:" sürücü harfi kodlanmamalı
        } else {
            out.push_str(&urlencoding::encode(seg));
        }
    }
    out
}

/// Tam bir HTML belgesini geçici dosyaya yazar, sistem tarayıcısında açar ve
/// yazdırma penceresini otomatik tetikler (kullanıcı oradan "PDF olarak kaydet" der).
#[tauri::command]
fn print_html(html: String, name: Option<String>) -> Result<String, String> {
    let mut dir = std::env::temp_dir();
    dir.push("plotica-print");
    std::fs::create_dir_all(&dir).map_err(|e| format!("gecici klasor acilamadi: {e}"))?;

    // 24 saatten eski baskı dosyalarını temizle
    let now = std::time::SystemTime::now();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if let Ok(md) = e.metadata() {
                if let Ok(m) = md.modified() {
                    if now.duration_since(m).map(|d| d.as_secs() > 86_400).unwrap_or(false) {
                        let _ = std::fs::remove_file(e.path());
                    }
                }
            }
        }
    }

    let stamp = now
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    let base = name
        .map(|n| sanitize_file_name(&n))
        .filter(|n| !n.is_empty() && n != "Adsiz")
        .unwrap_or_else(|| format!("plotica_{stamp}"));
    let path = dir.join(format!("{base}.html"));

    // Otomatik yazdırma betiğini ekle (belge zaten tam HTML)
    let auto = "<script>window.addEventListener('load',function(){setTimeout(function(){window.print();},400);});</script>";
    let page = if let Some(idx) = html.rfind("</body>") {
        let mut s = String::with_capacity(html.len() + auto.len());
        s.push_str(&html[..idx]);
        s.push_str(auto);
        s.push_str(&html[idx..]);
        s
    } else {
        format!("{html}{auto}")
    };

    let with_meta = if page.contains("charset") {
        page
    } else {
        page.replacen("<head>", "<head><meta charset=\"utf-8\">", 1)
    };

    std::fs::write(&path, with_meta.as_bytes()).map_err(|e| format!("yazilamadi: {e}"))?;

    let url = file_url(&path);
    webbrowser::open(&url)
        .map_err(|e| format!("Tarayici acilamadi ({e}). Dosya: {}", path.display()))?;

    Ok(path.to_string_lossy().into_owned())
}

/* ═══ DOĞRUDAN PDF (yazdırma penceresi ve tarayıcı üstbilgisi olmadan) ═══
   WebView2'nin PrintToPdf API'si üstbilgi/altbilgi basmaz (varsayılan kapalı).
   HTML gizli bir pencerede yüklenir, sayfa hazır olunca PDF'e yazılır. */
#[cfg(windows)]
#[tauri::command]
async fn export_pdf(
    app: tauri::AppHandle,
    html: String,
    suggested_name: String,
) -> Result<Option<String>, String> {
    use std::sync::mpsc;
    use tauri_plugin_dialog::DialogExt;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PRINT_ORIENTATION_PORTRAIT, ICoreWebView2Environment6,
        ICoreWebView2PrintSettings, ICoreWebView2_7,
    };
    use webview2_com::PrintToPdfCompletedHandler;
    use windows::core::{Interface, HSTRING, PCWSTR};

    // 1) Kullanıcıdan hedef dosyayı sor
    let picked = app
        .dialog()
        .file()
        .set_title("PDF olarak kaydet")
        .set_file_name(&sanitize_file_name(&suggested_name))
        .add_filter("PDF belgesi (*.pdf)", &["pdf"])
        .blocking_save_file();
    let Some(fp) = picked else { return Ok(None) };
    let mut out = fp.into_path().map_err(|e| e.to_string())?;
    if out.extension().is_none() {
        out.set_extension("pdf");
    }

    // 2) HTML'i geçici dosyaya yaz
    let mut dir = std::env::temp_dir();
    dir.push("plotica-print");
    std::fs::create_dir_all(&dir).map_err(|e| format!("gecici klasor: {e}"))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    let hpath = dir.join(format!("pdf_{stamp}.html"));
    std::fs::write(&hpath, html.as_bytes()).map_err(|e| format!("html yazilamadi: {e}"))?;

    let url_str = file_url(&hpath);
    let url: tauri::Url = url_str.parse().map_err(|e| format!("url: {e}"))?;

    // 3) Gizli pencerede yükle, sayfa bitince PDF'e bas
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    let out_for_cb = out.clone();
    let label = format!("pdfprint_{stamp}");

    let win = tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::External(url))
        .visible(false)
        .title("Plotica PDF")
        .inner_size(900.0, 1200.0)
        .on_page_load(move |w, payload| {
            if !matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                return;
            }
            let tx = tx.clone();
            let target = out_for_cb.clone();
            let _ = w.with_webview(move |pw| {
                let tx_err = tx.clone();
                let res: windows::core::Result<()> = (|| unsafe {
                    let core = pw.controller().CoreWebView2()?;
                    let wv7: ICoreWebView2_7 = core.cast()?;
                    let env6: ICoreWebView2Environment6 = pw.environment().cast()?;
                    let s: ICoreWebView2PrintSettings = env6.CreatePrintSettings()?;

                    // ANAHTAR: tarayıcı üstbilgi/altbilgisi (tarih, başlık, URL) basılmaz
                    s.SetShouldPrintHeaderAndFooter(false)?;
                    s.SetOrientation(COREWEBVIEW2_PRINT_ORIENTATION_PORTRAIT)?;
                    s.SetPageWidth(8.5)?;
                    s.SetPageHeight(11.0)?;
                    s.SetMarginTop(0.0)?;
                    s.SetMarginBottom(0.0)?;
                    s.SetMarginLeft(0.0)?;
                    s.SetMarginRight(0.0)?;
                    s.SetScaleFactor(1.0)?;
                    s.SetShouldPrintBackgrounds(true)?;
                    s.SetShouldPrintSelectionOnly(false)?;

                    let hs = HSTRING::from(target.to_string_lossy().as_ref());
                    let handler = PrintToPdfCompletedHandler::create(Box::new(move |hr, ok| {
                        let r = match (hr, ok) {
                            (Ok(()), true) => Ok(()),
                            (Ok(()), false) => Err("PDF yazilamadi".to_string()),
                            (Err(e), _) => Err(format!("PrintToPdf: {e}")),
                        };
                        let _ = tx.send(r);
                        Ok(())
                    }));
                    wv7.PrintToPdf(PCWSTR(hs.as_ptr()), &s, &handler)?;
                    Ok(())
                })();
                if let Err(e) = res {
                    let _ = tx_err.send(Err(format!("WebView2: {e}")));
                }
            });
        })
        .build()
        .map_err(|e| format!("pencere: {e}"))?;

    // 4) Sonucu bekle (ana thread'i kilitlememek için ayrı thread'de)
    let result = tauri::async_runtime::spawn_blocking(move || {
        rx.recv_timeout(std::time::Duration::from_secs(60))
    })
    .await
    .map_err(|e| e.to_string())?;

    let _ = win.close();
    let _ = std::fs::remove_file(&hpath);

    match result {
        Ok(Ok(())) => Ok(Some(out.to_string_lossy().into_owned())),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("PDF olusturma zaman asimina ugradi".to_string()),
    }
}

/// macOS/Linux: WebView2 yok. JS bu hatayı yakalayıp yazdırma penceresine düşer.
#[cfg(not(windows))]
#[tauri::command]
async fn export_pdf(
    _app: tauri::AppHandle,
    _html: String,
    _suggested_name: String,
) -> Result<Option<String>, String> {
    Err("dogrudan-pdf-bu-platformda-yok".to_string())
}

// Dış linkleri sistem tarayıcısında aç (Tauri webview'i doğrudan açmaz).
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://") || url.starts_with("mailto:")) {
        return Err("gecersiz url".to_string());
    }
    webbrowser::open(&url).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![google_login, open_url, save_text_file, print_html, export_pdf])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
