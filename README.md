# Plotica — Masaüstü (Tauri v2)

Plotica senaryo uygulamasının masaüstü sürümü. Tek dosyalık web uygulaması (`src/index.html`)
Tauri ile native bir pencereye sarılır. Google girişi **sistem tarayıcısında** açılır
(loopback OAuth), çünkü Tauri webview'i Firebase'in popup/redirect akışını çalıştıramaz.

```
plotica-tauri/
├─ src/
│  └─ index.html          ← uygulama (düzeltilmiş sürüm + masaüstü giriş kancası)
├─ src-tauri/
│  ├─ Cargo.toml
│  ├─ build.rs
│  ├─ tauri.conf.json
│  ├─ app-icon.png        ← yer tutucu ikon (kendi ikonunla değiştir)
│  ├─ capabilities/default.json
│  └─ src/
│     ├─ main.rs
│     └─ lib.rs           ← loopback OAuth akışı (Rust)
└─ README.md
```

---

## 1) Gereksinimler

| Araç | Not |
|------|-----|
| **Rust** (rustup) | https://rustup.rs |
| **Tauri CLI** | `cargo install tauri-cli --version "^2.0.0"` |
| **WebView2** | Windows 11'de zaten kuruludur (Edge ile gelir) |
| Visual Studio Build Tools (C++) | Windows'ta Rust derlemesi için gerekir |

> Not: Node.js **gerekmez** — frontend tek HTML dosyası, build adımı yok.

## 2) İkonları oluştur (ilk kurulumda bir kez)

`tauri.conf.json` ikon dosyalarına referans verir; derleme için bunların var olması gerekir.
Kendi kare PNG'ni (ideal 1024×1024) koy veya hazır yer tutucuyu kullan:

```powershell
cd "plotica-tauri"
cargo tauri icon src-tauri/app-icon.png
```

Bu komut `src-tauri/icons/` altına gereken tüm formatları (`.ico`, `.icns`, `.png`) üretir.

## 3) Geliştirme / Derleme

```powershell
cd "plotica-tauri"

# Geliştirme penceresi (canlı):
cargo tauri dev

# Dağıtılabilir kurulum dosyası (.msi / .exe) üretir:
cargo tauri build
```

Çıktı: `src-tauri/target/release/bundle/` altında.

---

## 4) Firebase + Google Cloud kurulumu (giriş için)

Masaüstü girişi iki kimlik kullanır:
1. **Firebase Web yapılandırması** (apiKey, projectId) — Firestore sync için.
2. **Google "Masaüstü uygulaması" OAuth istemcisi** (Client ID + Secret) — girişin kendisi için.

### Adımlar
1. [Firebase Console](https://console.firebase.google.com) → projeni aç (yoksa oluştur).
2. **Authentication → Sign-in method → Google**'ı etkinleştir.
3. **Firestore Database**'i oluştur.
4. [Google Cloud Console](https://console.cloud.google.com) → **aynı proje** → **APIs & Services → Kimlik Bilgileri (Credentials)**:
   - **Kimlik bilgisi oluştur → OAuth istemci kimliği → Uygulama türü: "Masaüstü uygulaması"** seç.
   - Oluşan **Client ID** ve **Client Secret**'i kopyala.
5. Uygulamayı çalıştır → giriş ekranında **⚙️ "Firebase ile buluta bağla"** → şunları gir:
   - apiKey, projectId
   - **OAuth Client ID** ve **OAuth Client Secret** (4. adımdan)
   - Kaydet → **"Google ile Giriş Yap"**.

Tarayıcı açılır, izin verirsin, sekme "Giriş tamamlandı" der, uygulama otomatik bağlanır.

---

## 5) ❓ "Authorized domains'e ne ekleyeceğiz?" — kısa cevap

**Firebase Authorized Domains'e Tauri için HİÇBİR ŞEY eklemene gerek yok — ekleyemezsin de.**

Nedeni:
- Tauri webview'inin origin'i Windows'ta `https://tauri.localhost`, diğer platformlarda
  `tauri://localhost`'tur. Bunlar gerçek alan adı değildir; **Firebase Authorized Domains
  listesi yalnızca gerçek domainleri ve `localhost`'u kabul eder.** Bu yüzden bu origin'i
  ekleyemezsin — ve zaten popup/redirect akışı bu sebeple masaüstünde çalışmaz.
- Bu sürümde giriş **gerçek sistem tarayıcısında** `accounts.google.com` üzerinde olur ve
  yönlendirme `http://127.0.0.1:<port>`'a döner. `signInWithCredential` kullandığımız için
  Firebase'in "authorized domains" kontrolü **devreye girmez**.

Yani:

| Ayar | Nereye | Ne eklenecek |
|------|--------|--------------|
| Firebase → Authentication → Settings → **Authorized domains** | — | **Hiçbir şey.** `localhost` zaten varsayılan ekli; web sürümünü de kullanıyorsan kendi web domainini ekle. |
| Google Cloud → Credentials → **OAuth istemcisi** | İstemci türü | **"Masaüstü uygulaması"** seç — bu tür `http://127.0.0.1` ve `http://localhost`'u **her portta** otomatik kabul eder, redirect URI yazmana gerek yok. |

> Eğer (yanlışlıkla) **"Web uygulaması"** türünde bir OAuth istemcisi oluşturursan, rastgele
> loopback portları için tek tek redirect URI eklemen gerekir ki bu pratik değildir.
> Mutlaka **"Masaüstü uygulaması"** türünü seç.

---

## 6) Nasıl çalışıyor? (akış)

```
[Plotica penceresi]  --invoke('google_login')-->  [Rust]
                                                     │ 1. PKCE üret, 127.0.0.1:<port> dinle
                                                     │ 2. SİSTEM TARAYICISINI aç (Google izin ekranı)
[Sistem tarayıcısı] --izin--> Google --redirect--> http://127.0.0.1:<port>/?code=...
                                                     │ 3. code'u yakala
                                                     │ 4. code -> token (oauth2.googleapis.com)
[Plotica] <--{id_token, access_token}-------------- [Rust]
   │ firebase.auth().signInWithCredential(...)
   └─> onAuthStateChanged -> Firestore canlı sync
```

İlgili kod: `src-tauri/src/lib.rs` (`google_login`) ve `src/index.html` (`loginWithGoogleDesktop`).

## 7) Güvenlik notları

- Masaüstü OAuth istemcisinin Client Secret'i, kurulu uygulamalarda Google tarafından
  "gizli sayılmaz"; gerçek koruma **PKCE** ile sağlanır (bu sürüm PKCE kullanır).
- `tauri.conf.json` içinde `security.csp: null` ayarı, Firebase SDK ve Google Fonts'un
  CDN'den yüklenebilmesi için CSP'yi kapatır. Daha sıkı bir kurulum istersen CSP'yi
  gerekli origin'lerle (gstatic.com, googleapis.com, firestore) sınırlayabilirsin.
- Offline mod giriş gerektirmez; veriler `localStorage`'da saklanır.
