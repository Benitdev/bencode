# BenCode ⚡

**BenCode** là control plane dạng desktop cho các coding agent CLI (Claude Code,
Codex, Antigravity, OpenCode), viết 100% bằng **Rust** với **Zed GPUI** và
**[Ely GPUI Components](https://elygpui.com/)**.

Đây là bản port native của **MonoCode** (Tauri + React): cùng tính năng, cùng
giao diện, cùng schema cơ sở dữ liệu, nhưng không có WebView.

**[Tải BenCode cho macOS](https://github.com/Benitdev/bencode/releases/latest/download/BenCode.dmg)**
· [Trang giới thiệu](https://benitdev.github.io/bencode/)
· [Releases](https://github.com/Benitdev/bencode/releases)

---

## Mục tiêu

1. **Native hoàn toàn**: không Electron, không Chromium, không WebKit. Mọi thứ
   được GPUI vẽ trực tiếp bằng GPU (Metal trên macOS).
2. **Nhanh và nhẹ**: mục tiêu khởi động dưới 50ms và dùng khoảng 30MB RAM.
3. **Local-first, dữ liệu riêng**: thread, checkpoint và account nằm trong thư
   mục của BenCode. Lần chạy đầu, dữ liệu của MonoCode (nếu có) được sao chép
   sang một lần; sau đó hai ứng dụng độc lập.
4. **Không tốn thêm token**: BenCode chỉ điều khiển CLI qua stdio và đọc luồng
   JSON chúng in ra.

---

## Tính năng

| Khu vực | Có gì |
| :--- | :--- |
| **Chat** | Transcript theo lượt (câu trả lời, reasoning, tool call), nhiều pane chia đôi, tìm trong hội thoại (⌘F) |
| **Composer** | Chọn model, chế độ quyền, nhánh / worktree; `@` nhắc file, `/` gọi skill, `/mcp`; đính kèm file; handoff sang agent khác; sửa và gửi lại lượt cuối |
| **Agent** | Claude Code, Antigravity (`agy`), Codex, OpenCode; hỏi quyền chạy tool ngay trong transcript; theo dõi token và giới hạn sử dụng |
| **File** | Explorer (tạo, đổi tên, copy, cut, paste, xoá), trình soạn code có lưu atomic và phát hiện file đổi trên đĩa, Go to File (⌘P) |
| **Git** | Staged / unstaged, commit, sinh commit message, fetch / pull / push, tạo PR, đồ thị commit, worktree |
| **Session review** | Sau mỗi lượt có sửa file: card "Changed N files" với Undo, Keep và Review riêng cho thay đổi của thread đó |
| **Review** | Diff của working tree và của commit: các file xếp chồng, header dính, gập đoạn không đổi, stage / discard ngay trên header |
| **Terminal** | Terminal native theo từng project (⌘J) |
| **Inbox** | Issue và pull request GitHub qua `gh`: checks, comment, merge, nhờ agent sửa CI. Issue Nulab Backlog qua API key (Settings › Integrations): comment, đổi status, giao cho agent |
| **Notes** | Ghi chú markdown, tag, gắn với thread |
| **Automations** | Prompt chạy theo lịch, lịch sử chạy |
| **Khác** | Tìm kiếm toàn cục (⌘K), nhắc việc theo thread, thư mục thread, MCP server, mở bằng editor ngoài |

Chat luôn hiển thị. File, diff và commit mở thành tab trong một pane bên phải
chat; đóng tab cuối thì chat lấy lại toàn bộ chiều rộng.

---

## Cài đặt

1. Tải [`BenCode.dmg`](https://github.com/Benitdev/bencode/releases/latest/download/BenCode.dmg)
   (một bản universal cho cả Apple Silicon và Intel, macOS 11 trở lên).
2. Mở file và kéo **BenCode** vào **Applications**.
3. Bản build chưa được Apple notarize, nên lần mở đầu macOS sẽ hỏi lại: mở
   BenCode một lần, rồi vào **System Settings › Privacy & Security › Open
   Anyway**. Nếu macOS báo app "is damaged", chạy:

   ```bash
   xattr -dr com.apple.quarantine /Applications/BenCode.app
   ```

Gặp lỗi? **Help › Show Logs** mở file log để đính kèm vào
[issue](https://github.com/Benitdev/bencode/issues).

---

## Yêu cầu

- **macOS 11** trở lên (nền tảng chính; dùng Metal và API Cocoa).
- **git** trong `PATH`.
- Ít nhất một agent CLI đã cài và đăng nhập: `claude`, `agy`, `codex` hoặc `opencode`.
- Tuỳ chọn: **`gh`** (GitHub CLI) cho Inbox và pull request.
- Chỉ khi build từ mã nguồn: **Rust** bản stable mới (edition 2024) cùng Xcode
  Command Line Tools.

---

## Chạy từ mã nguồn

```bash
git clone https://github.com/Benitdev/bencode.git
cd bencode
cargo run
```

Lần build đầu mất vài phút vì phải biên dịch GPUI. Các lệnh hay dùng:

```bash
cargo check                  # kiểm tra kiểu, nhanh
cargo test                   # unit test
cargo run                    # chạy ứng dụng
cargo build --release        # bản tối ưu
packaging/macos/bundle.sh    # đóng gói BenCode.app và BenCode.dmg (target/bundle)
RUST_LOG=debug cargo run     # bật log
RUST_BACKTRACE=1 cargo run   # in backtrace khi panic
```

> **Lưu ý:** `cargo run` mở dữ liệu thật của BenCode (`bencode.db` bên dưới),
> giống bản đã cài. Thread, ghi chú và automation sửa ở đây là thật.

---

## Dữ liệu nằm ở đâu

| Dữ liệu | Đường dẫn |
| :--- | :--- |
| Thread, block, ghi chú, automation, nhắc việc | `~/Library/Application Support/BenCode/bencode.db` |
| Thiết lập riêng của BenCode | `~/Library/Application Support/BenCode/settings.json` |
| Checkpoint để xem lại và hoàn tác thay đổi của agent | `~/Library/Application Support/BenCode/checkpoints` |
| Thư mục cấu hình của từng account provider | `~/Library/Application Support/BenCode/provider-accounts` |
| Log (khi không chạy từ terminal) | `~/Library/Logs/BenCode/bencode.log` |

BenCode không dùng chung dữ liệu nào với MonoCode. Lần chạy đầu tiên chưa có
`bencode.db`, nó sao chép một lần database, checkpoints và account profiles của
MonoCode (nếu có) sang thư mục trên; MonoCode chỉ bị đọc, không bị sửa.

---

## Phím tắt

| Phím | Tác dụng |
| :--- | :--- |
| ⌘T | Thread mới |
| ⌘O | Mở project |
| ⌘K | Tìm kiếm |
| ⌘P | Go to File |
| ⌘F / ⌘G / ⇧⌘G | Tìm trong hội thoại / kết quả sau / trước |
| ⌘. | Đổi model |
| ⌘S | Lưu file đang mở |
| ⌘W | Đóng tab của pane file (nếu đang thao tác ở đó), nếu không thì đóng thread |
| ⌘B / ⇧⌘B | Ẩn hiện rail project / sidebar |
| ⌘J / ⌘` | Ẩn hiện terminal / terminal mới |
| ⌘D / ⇧⌘D | Chia pane sang phải / xuống dưới |
| ⌥⌘←→↑↓ | Chuyển focus giữa các pane |
| ⇧⌘] / ⇧⌘[ | Tab kế tiếp / trước |
| ⌘] / ⌘[ | Tới / lui theo lịch sử tab |
| ⌘, | Settings |
| Esc | Đóng view đang mở |

Toàn bộ phím tắt và menu được khai báo ở `src/app/commands.rs`.

---

## Kiến trúc

```
bencode/
├── Cargo.toml
├── AGENTS.md            hướng dẫn chi tiết cho developer và AI agent
├── CHANGELOG.md         thay đổi theo từng bản phát hành
├── assets/              icon SVG (Lucide bổ sung, logo provider)
├── docs/migration/      backlog so khớp với MonoCode
├── docs/releasing.md    cách phát hành một bản mới
├── packaging/macos/     icon app, Info.plist, script đóng gói .app / .dmg
├── site/                landing page (GitHub Pages)
├── tests/fixtures/      output CLI ghi lại để test parser
└── src/
    ├── main.rs          cửa sổ, theme, keymap
    ├── app.rs           BenCodeApp: entity duy nhất giữ state
    ├── app/             logic theo từng mảng (agent, pane, workspace, settings…)
    ├── ui/              toàn bộ view
    ├── harness/         điều khiển agent CLI qua stdio
    ├── db/              SQLite của BenCode (schema của MonoCode)
    ├── git/             git qua CLI (status, diff, sync, graph, worktree, checkpoint)
    ├── github.rs        GitHub qua `gh`
    ├── mcp/, skills/    MCP server và SKILL.md
    ├── schedule.rs      lịch chạy automation
    └── settings.rs      settings.json
```

Luồng một lượt chat:

```
prompt ─► app/agent.rs ─► harness::spawn ─► tiến trình CLI
                                               │ stdout (JSON từng dòng)
                                               ▼
                               LineParser ─► AgentEvent
                                               │
              transcript ◄── session.blocks ◄──┘──► SQLite
```

Vài nguyên tắc cốt lõi:

- **Không IO trong `render()`** và không chặn UI thread: git, đĩa, SQLite chạy
  trên background executor của GPUI; view chỉ đọc cache (`self.workspace`).
- **Tokio chỉ dành cho tiến trình harness** (`src/harness/runtime.rs`); executor
  của GPUI không có Tokio reactor.
- **Dùng component của Ely** thay vì tự viết; màu lấy từ `cx.theme().colors`.
- **Giữ nguyên dữ liệu không hiểu** khi ghi DB: dòng sao chép từ MonoCode mang
  theo cả những cột BenCode chưa dùng.

Chi tiết đầy đủ, kèm các quy tắc UI và lỗi thường gặp, nằm trong
[`AGENTS.md`](AGENTS.md).

---

## Stack

| Thành phần | Công nghệ |
| :--- | :--- |
| Ngôn ngữ | Rust (edition 2024) |
| UI | [GPUI](https://www.gpui.rs/) của Zed |
| Component | [Ely GPUI Components](https://github.com/ZacharyZhang-NY/Ely-GPUI-Components) |
| Cơ sở dữ liệu | SQLite qua `rusqlite` (bundled) |
| Tiến trình agent | Tokio |
| Thời gian, băm, JSON | `jiff`, `sha2`, `serde_json` |

---

## Đóng góp

1. Đọc [`AGENTS.md`](AGENTS.md) trước khi sửa code.
2. Khi port một tính năng, đối chiếu mã nguồn MonoCode trong `reference/monocode`
   (symlink, không nằm trong git).
3. Chạy `cargo test`, rồi chạy ứng dụng và thử trực tiếp phần vừa sửa.
4. Việc còn thiếu so với MonoCode được ghi ở `docs/migration/PARITY-BACKLOG.md`.

## Phát hành

Đẩy tag `vX.Y.Z` (trùng version trong `Cargo.toml`) là GitHub Actions build
`BenCode.dmg` và tạo GitHub Release. Các bước, cùng cách bật ký và notarize khi
có Apple Developer ID, nằm trong [`docs/releasing.md`](docs/releasing.md).

## Giấy phép

MIT, xem [`LICENSE`](LICENSE). Icon Lucide trong `assets/icons` (ISC) và
Material Icon Theme trong `assets/file-icons` (MIT) giữ giấy phép riêng ở thư
mục của chúng; logo các provider thuộc về chủ sở hữu tương ứng.
