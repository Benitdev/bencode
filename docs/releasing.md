# Phát hành BenCode

BenCode được phát hành qua **GitHub Releases** và **landing page** trên GitHub
Pages, không qua App Store. Mỗi bản là một file `BenCode.dmg` universal (Apple
Silicon và Intel, macOS 11 trở lên).

| Thành phần | Ở đâu |
| :--- | :--- |
| Icon app (nguồn SVG và PNG 1024) | `packaging/macos/icon.svg`, `icon-1024.png` |
| `Info.plist`, entitlements | `packaging/macos/` |
| Script đóng gói `.app` / `.dmg` | `packaging/macos/bundle.sh` |
| Workflow phát hành (tag `v*`) | `.github/workflows/release.yml` |
| Landing page | `site/index.html`, deploy bằng `.github/workflows/pages.yml` |
| Ghi chú phát hành | `CHANGELOG.md` |
| Tự cập nhật (app) | `src/updater.rs`, `src/app/updater.rs`, `src/ui/rail/update.rs` |

---

## Thiết lập một lần

1. **Bật GitHub Pages:** repo › Settings › Pages › *Build and deployment* ›
   Source: **GitHub Actions**. Sau đó chạy workflow **Pages** (Actions › Pages
   › Run workflow), hoặc push một thay đổi trong `site/` lên `main`. Trang nằm
   ở `https://benitdev.github.io/bencode/`.
2. **Bundle ID** là `com.benitdev.bencode` (`packaging/macos/Info.plist`).
   Nếu muốn đổi thì đổi trước bản đầu tiên: macOS gắn các quyền đã cấp
   (Documents, Desktop…) theo bundle ID.
3. **Khoá cập nhật** để app tự cập nhật, xem mục "Tự cập nhật" bên dưới.
   Chưa có khoá thì app vẫn chạy, chỉ là không tự cập nhật.
4. **Ký và notarize** là tuỳ chọn, xem mục cuối.

---

## Mỗi lần phát hành

1. Tăng `version` trong `Cargo.toml`, rồi chạy `cargo check` để `Cargo.lock`
   cập nhật theo.
2. Trong `CHANGELOG.md`, đổi `## [x.y.z] - Unreleased` thành ngày phát hành
   (`## [0.1.0] - 2026-10-10`) và liệt kê thay đổi. Đoạn này thành release
   notes; thiếu đoạn này thì GitHub tự liệt kê commit.
3. Commit và merge vào `main`.
4. Gắn tag và push:

   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```

5. Theo dõi **Actions › Release**. Workflow build hai kiến trúc với LTO nên
   mất khoảng 30 đến 60 phút. Xong thì Release `v0.1.0` có `BenCode.dmg` và
   `BenCode.dmg.sha256`.
6. Kiểm tra trên một máy khác (hoặc một user macOS mới): tải từ landing page,
   kéo vào Applications, mở lần đầu, chạy một lượt với agent, mở terminal,
   Help › Show Logs.

Workflow dừng ngay nếu tag không khớp version trong `Cargo.toml`.

**Pre-release:** tag có dấu gạch (`v0.2.0-beta.1`) được đánh dấu pre-release.
Link `releases/latest/download/BenCode.dmg` trên landing page bỏ qua
pre-release, nên vẫn trỏ tới bản ổn định mới nhất.

**Build thử mà không phát hành:** Actions › Release › Run workflow. Mặc định
bản thử chỉ build Apple Silicon và bật *Quick build* (thin LTO, compile song
song), nên xong nhanh hơn nhiều; chọn cả hai kiến trúc và bỏ *Quick build* để
build y như bản phát hành. File dmg nằm trong mục *Artifacts* của lần chạy đó.

**Build trên máy:**

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
packaging/macos/bundle.sh                              # universal
TARGETS=aarch64-apple-darwin packaging/macos/bundle.sh # chỉ Apple Silicon, nhanh hơn
open target/bundle/BenCode.dmg
```

**Đổi icon:** sửa `packaging/macos/icon.svg`, rồi render lại PNG:

```bash
NODE_PATH="$(npm root -g)" node packaging/macos/render-icon.mjs   # cần playwright
```

---

## Tự cập nhật

App kiểm tra bản mới một lần khi mở, và khi bấm **BenCode › Check for
Updates…** hoặc Settings › About. Có bản mới thì rail hiện nút "Update to X".
Bấm vào, app sẽ:

1. tải `BenCode.app.tar.gz` từ Release mới nhất;
2. kiểm tra chữ ký minisign bằng public key được nhúng lúc build;
3. kiểm tra bundle ID, version và `codesign --verify`;
4. thay `BenCode.app` đang chạy bằng bản mới rồi tự khởi động lại.

Lần mở sau, rail hiện thẻ "Updated to X / What's new". Ghi chú lấy từ
`CHANGELOG.md` được nhúng trong app.

App đọc `https://github.com/Benitdev/bencode/releases/latest/download/latest.json`
(định dạng `latest.json` của Tauri). `bundle.sh` sinh file này cùng archive
và chữ ký; workflow Release đăng cả ba lên mỗi Release.

**Tạo khoá (một lần, trên máy bạn):**

```bash
brew install minisign
minisign -G -W -p bencode-update.pub -s bencode-update.key   # -W: không đặt mật khẩu
```

Rồi vào repo › Settings › Secrets and variables › Actions:

| Loại | Tên | Giá trị |
| :--- | :--- | :--- |
| Variable | `BENCODE_UPDATE_PUBKEY` | dòng thứ hai của `bencode-update.pub` (chuỗi bắt đầu bằng `RW`) |
| Secret | `MINISIGN_SECRET_KEY` | toàn bộ nội dung `bencode-update.key` |

Phải đặt cả hai hoặc không đặt cái nào; đặt một nửa thì workflow dừng với
lỗi. Cất `bencode-update.key` ở nơi an toàn (password manager) và đừng commit
nó. Mất khoá thì các bản đã cài không còn nhận cập nhật được nữa, người dùng
phải tải bản mới bằng tay.

**Lưu ý:**

- Chỉ bản build có `BENCODE_UPDATE_PUBKEY` mới tự cập nhật. `cargo run`
  không có khoá: "Check for Updates…" báo build này không tự cập nhật và đưa
  link Releases.
- App phải nằm ở chỗ ghi được (thường là `/Applications`). Nếu macOS đang
  chạy BenCode từ bản sao tạm (App Translocation, khi mở thẳng từ Downloads)
  hoặc từ file dmg, app sẽ nhắc kéo vào Applications trước.
- Bản tải bằng `curl` không bị gắn cờ quarantine, nên khi cập nhật không phải
  bấm "Open Anyway" lại.

---

## Khi chưa có Apple Developer ID (hiện tại)

Không có secret nào thì app được ký ad hoc. Người dùng tải về phải xác nhận
lần mở đầu (System Settings › Privacy & Security › Open Anyway, hoặc
`xattr -dr com.apple.quarantine /Applications/BenCode.app`). Release notes,
README và landing page đã ghi hướng dẫn này.

Bản ký ad hoc mang chữ ký khác nhau giữa các lần build, nên sau mỗi lần cập
nhật macOS có thể hỏi lại các quyền đã cấp (Documents, Desktop…).

## Bật ký và notarize

Cần tham gia Apple Developer Program (99 USD/năm). Không liên quan tới App
Store: Developer ID chỉ để Gatekeeper tin bản tải từ web.

1. Tạo chứng chỉ **Developer ID Application** (Xcode › Settings › Accounts ›
   Manage Certificates, hoặc developer.apple.com › Certificates), rồi export
   từ Keychain Access thành file `.p12` có mật khẩu.
2. Tạo app-specific password ở [account.apple.com](https://account.apple.com)
   › Sign-In and Security › App-Specific Passwords.
3. Thêm các secret ở repo › Settings › Secrets and variables › Actions:

   | Secret | Giá trị |
   | :--- | :--- |
   | `MACOS_CERTIFICATE` | `base64 -i DeveloperID.p12 \| pbcopy` |
   | `MACOS_CERTIFICATE_PASSWORD` | mật khẩu của file `.p12` |
   | `APPLE_ID` | email Apple ID của tài khoản developer |
   | `APPLE_TEAM_ID` | Team ID 10 ký tự (developer.apple.com › Membership) |
   | `APPLE_APP_PASSWORD` | app-specific password ở bước 2 |

4. Từ bản kế tiếp, workflow ký app bằng hardened runtime cùng
   `packaging/macos/entitlements.plist`, notarize và staple file dmg, và bỏ
   đoạn "First launch" khỏi release notes.
5. Bỏ bước "Confirm the first launch" trên landing page (`#first-launch` trong
   `site/index.html`) và bước 3 của mục Cài đặt trong README.

Lần đầu bật ký, hãy chạy thử bản đã ký (Actions › Release › Run workflow):
mở app, chạy một lượt agent, mở terminal, thử một lệnh cần quyền (ví dụ `ls
~/Documents`). Nếu hardened runtime chặn thứ gì, thêm entitlement tương ứng
vào `entitlements.plist`.
