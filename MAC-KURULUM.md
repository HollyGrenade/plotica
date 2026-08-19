# Plotica — macOS Sürümü

Uygulamanın tamamı platform bağımsız yazıldı. macOS için **kod değişikliği gerekmiyor**;
sadece bir Mac üzerinde (veya GitHub'ın Mac sunucusunda) derlemek yeterli.

> **Neden burada derleyemiyoruz?** Tauri, macOS uygulamasını ancak macOS üzerinde
> (Xcode araçlarıyla) derleyebilir. Windows'tan Mac'e çapraz derleme mümkün değil.

---

## Yol A — Mac'in varsa (en hızlı)

Mac'te Terminal aç:

```bash
# 1) Gereksinimler (bir kez)
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
cargo install tauri-cli --version "^2.0.0" --locked

# 2) Proje klasörüne gir (plotica-tauri klasörünü Mac'e kopyala)
cd ~/plotica-tauri

# 3) Derle
cargo tauri build
```

Çıktı: `src-tauri/target/release/bundle/dmg/Plotica_1.0.0_*.dmg`
Çift tıkla → Uygulamalar'a sürükle → bitti.

Geliştirme için: `cargo tauri dev`

---

## Yol B — Mac'in yoksa (GitHub üzerinden, ücretsiz)

`.github/workflows/build.yml` hazır. Adımlar:

1. Bu klasörü bir GitHub deposuna yükle (özel/private olabilir)
2. Depoda **Actions** sekmesi → **"Plotica derleme"** → **Run workflow**
3. ~10–15 dk sonra **Artifacts** bölümünden indir:
   - `Plotica-macOS-AppleSilicon` → M1/M2/M3/M4 Mac'ler için `.dmg`
   - `Plotica-macOS-Intel` → Intel Mac'ler için `.dmg`
   - `Plotica-Windows` → Windows kurulumu

---

## macOS'ta farklı çalışan tek şey

| Özellik | Windows | macOS |
|---|---|---|
| Giriş, senkron, sayfalı editör, TXT/FDX/CSV kaydetme | ✅ | ✅ aynı |
| Dış linkler (Destekle) | ✅ | ✅ aynı |
| **PDF çıktısı** | Doğrudan yazılır (yazdırma penceresi yok) | Yazdırma penceresi açılır → **PDF → Farklı Kaydet** |

Sebep: doğrudan PDF yazma, Windows'un WebView2 motoruna özgü bir özellik.
macOS'ta uygulama otomatik olarak yazdırma penceresine düşer — çıktı aynı,
sadece bir ekstra tık var. macOS'un yazdırma penceresinde üstbilgi/altbilgi
varsayılan olarak **kapalıdır**, yani o tarih/URL satırları çıkmaz.

---

## İmzalama (isteğe bağlı, dağıtım için)

İmzasız `.dmg` açılırken macOS uyarı verir. Kullanıcı **sağ tık → Aç** ile geçebilir.
Uyarıyı tamamen kaldırmak için Apple Developer hesabı (yıllık $99) ile
imzalama + notarization gerekir. Kişisel kullanım için gerekmez.
