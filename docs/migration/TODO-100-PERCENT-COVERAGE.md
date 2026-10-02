# BenCode: Kế Hoạch & TODO List Hoàn Thiện 100% Tính Năng Từ MonoCode ⚡

> **Mục tiêu**: Chuyển đổi toàn diện MonoCode (Tauri v2 + React 19 + TypeScript) sang BenCode (100% Native Rust + Zed GPUI + Ely GPUI Components), đạt hiệu năng cực hạn (GPU Metal 120 FPS, khởi động <50ms, RAM ~30MB) và tương thích hoàn toàn hệ sinh thái dữ liệu của MonoCode.

---

## 📊 Tổng Quan Hiện Trạng (Audit Summary)

- **Đã hoàn thành (~70%)**:
  - Khung kiến trúc GPUI 100% Native Rust, tích hợp thư viện [Ely GPUI Components](https://ely-gpui.zacharyzhang.com/).
  - Hệ thống giao diện: Left Rail, Sidebar 3 chế độ (Sessions, File Tree, Git Changes), Titlebar, Transcript Timeline, Prompt Composer, Diff Viewer (virtualized line renderer), Terminal Pane, Settings Modal, Notes View, Automations View, Search Modal, Footer.
  - Tầng dữ liệu SQLite: Kết nối trực tiếp với DB của MonoCode (`monocode.db`), tương thích schema sessions, notes, automations.
  - Git CLI operations: Lấy status, branch, diff, commits, staging, unstage, discard.
  - Multi-Harness Live Process Spawning & Stdio Streaming: Antigravity, Claude, Codex, OpenCode với Interactive Tool Approval.
  - Git Worktree engine (`src/git/worktrees.rs`): list, create, prune, guarded remove. **Partial: engine ported, not wired into the app.**
  - Checkpoint & Undo engine (`src/git/checkpoint.rs`): file snapshotting & rollback (SHA-256 blobs, path/symlink guards, foreign-edit detection). **Partial: engine ported, not wired into the app.**
  - External Editors (`src/external_editor.rs`): Tự động phát hiện Cursor, VS Code, Zed, Windsurf và mở file/line trực tiếp.
  - MCP Discovery (`src/mcp/mod.rs`): Tự động quét cấu hình MCP từ `~/.claude.json`, Claude Desktop, Cursor, và project `.bencode/mcp.json` / `.mcp.json`. **Discovery only; no supervisor/JSON-RPC client.**

- **Đang tiếp tục triển khai (~30%)**:
  - Trình xem/chỉnh sửa code native (Native Code Editor) thay vì chỉ xem cây thư mục.
  - Quick Composer (Cửa sổ floating native mở bằng phím tắt toàn hệ thống).
  - Multi-Agent Orchestration (Đồ thị Constellation quản lý subagents).
  - Tích hợp Issue Tracker (GitHub, GitLab, Linear, Jira) & Remote SSH workspaces.
  - macOS Menu bar, System Tray, Dock Badge.

---

## 🗺️ Lộ Trình & TODO List Chi Tiết (Theo Mức Độ Ưu Tiên)

### 🔴 Phase 1: Core Agent Execution & Live Streaming (P0 - Cốt Lõi Bắt Buộc) - [HOÀN THÀNH ✅]
*Mục tiêu: Đảm bảo gõ prompt -> Agent thực thi thật trên máy -> Stream token/tool call về giao diện trong thời gian thực.*

- [x] **1.1 Live Agent Process Spawning & Stdio Streaming**
  - File: `src/app.rs` (`submit_prompt`), `src/harness/`
  - Thay thế mock string trong `submit_prompt` bằng việc spawn process thật (`HarnessDispatcher::spawn`).
  - Lắng nghe channel `mpsc::UnboundedReceiver<AgentEvent>` trong `cx.spawn()`, cập nhật block theo từng delta (`TextDelta`, `ThinkingDelta`).
  - Gọi `cx.notify()` mượt mà 120 FPS để render chữ xuất hiện theo thời gian thực.
- [x] **1.2 Nút Dừng / Cancel Agent (SIGINT/SIGTERM)**
  - File: `src/app.rs` (`handle_send_or_stop`), `src/harness/handle.rs`
  - Quản lý `HarnessProcessHandle`, gửi tín hiệu hủy (`handle.cancel()`) khi người dùng bấm Stop, giải phóng trạng thái và cập nhật UI tức thì.
- [x] **1.3 Multi-Harness Providers**
  - File: `src/harness/antigravity.rs`, `src/harness/claude.rs`, `src/harness/codex.rs`, `src/harness/opencode.rs`, `src/harness/mod.rs`
  - Triển khai adapter stdio stream-json cho Google Antigravity CLI (`agy -p ... --output-format stream-json`).
  - Triển khai adapter cho Claude Code CLI (`claude -p ... --output-format stream-json --verbose`).
  - Triển khai adapter cho OpenAI Codex CLI.
  - Triển khai adapter cho OpenCode CLI.
  - Tích hợp `HarnessDispatcher::spawn` tự động định tuyến theo harness của session.
- [x] **1.4 Chế Độ Phê Duyệt Tool Quyền Lực (Interactive Tool Approval)**
  - File: `src/ui/transcript.rs`, `src/app.rs`
  - Khi có yêu cầu cấp quyền tool (`PermissionRequest`), hiển thị card tương tác với nút `[Approve (y)]` và `[Deny (n)]`.
  - Gửi phản hồi `y\n` hoặc `n\n` trực tiếp qua stdin của process agent bằng `HarnessProcessHandle::send_input`.

---

### 🟠 Phase 2: Git Worktree Isolation & Checkpoint Snapshot (P1 - An Toàn Mã Nguồn) - [PARTIAL 🟡]
*Mục tiêu: Bảo vệ workspace của lập trình viên, cho phép rollback khi agent làm sai.*

- [ ] **2.1 Worktree Session Isolation** — partial: engine ported, not wired into the app
  - File: `src/git/worktrees.rs` (done), `src/app.rs` (not wired)
  - Done: `list_worktrees` (parallel status, `unpushed = None` without remotes, per-tree `status_error`), `create_worktree` (existing branch only via `refs/heads/<branch>`), `remove_worktree` (registered non-main target, refuses dirty / detached / unpushed unless `force`, `--` before path), `prune_worktrees`.
  - TODO (chưa làm):
    - Tự động tạo git worktree ngầm tại `.bencode/worktrees/<session-id>` khi bắt đầu phiên làm việc mới.
    - Trỏ `cwd` của agent vào worktree này để không làm bẩn working directory chính của lập trình viên.
    - Cung cấp thao tác "Merge to Main" hoặc "Discard Branch" khi phiên kết thúc.
- [ ] **2.2 Checkpoint & Undo Turn Engine** — partial: engine ported, not wired into the app
  - File: `src/git/checkpoint.rs` (done), `src/ui/transcript.rs` (not wired)
  - Done: `CheckpointStore::{prepare_file, capture_file, restore_file, restore_file_with, restore_all, restore_all_with, discard}` with id/path validation, symlink refusal, SHA-256 verified blobs, atomic manifest, mode preservation, restore pinned to `manifest.cwd`, foreign-edit refusal unless forced.
  - TODO (chưa làm):
    - Tự động tạo git stash/commit checkpoint trước mỗi turn người dùng nhập prompt.
    - Nút "Restore to this point" trên mỗi block trong transcript để khôi phục mã nguồn về trạng thái trước đó.

---

### 🟡 Phase 3: Native Editor & Nâng Cấp UI/UX (P1 - Trải Nghiệm Lập Trình)
*Mục tiêu: Trải nghiệm xem/sửa code, terminal và composer hoàn chỉnh.*

- [ ] **3.1 Trình Xem & Biên Tập Code Native (Native Code Editor)** — partial (being fixed)
  - File: `src/ui/editor_pane.rs`, `src/workspace.rs`
  - Tích hợp `ely_gpui_component::editor` để mở và chỉnh sửa file khi bấm vào file tree.
  - Hỗ trợ cú pháp highlight (Rust, TypeScript, Python, JSON, Markdown).
  - Phím tắt lưu file (`Cmd+S`), phát hiện định dạng dòng LF/CRLF.
- [x] **3.2 Mở File Bằng Trình Soạn Thảo Ngoài (External Editor)**
  - File: `src/external_editor.rs`, `src/ui/file_tree.rs`, `src/ui/diff_viewer.rs`, `src/ui/settings_modal.rs`
  - Tự động nhận diện Cursor, VS Code, Zed, Windsurf, Sublime Text cài đặt trên máy.
  - Thêm nút mở workspace / file tại đúng dòng từ File Tree, Diff Viewer, và hiển thị cấu hình trong Settings.
- [ ] **3.3 Image Paste & Đính Kèm File Trong Composer**
  - File: `src/ui/composer.rs`, `src/pasteboard.rs`
  - Bắt sự kiện dán ảnh từ clipboard (`Cmd+V`), lưu vào cache ảnh tạm và hiển thị thumbnail preview trong composer.
- [ ] **3.4 Nâng Cấp Terminal Pane**
  - File: `src/ui/terminal_pane.rs`
  - Hỗ trợ mở nhiều tab terminal, lựa chọn shell (Zsh, Bash, Fish), truyền đúng biến môi trường dự án.

---

### 🟢 Phase 4: MCP (Model Context Protocol) & Quick Composer (P2 - Tính Năng Cao Cấp)
*Mục tiêu: Mở rộng khả năng của Agent qua MCP và phím tắt toàn hệ thống.*

- [ ] **4.1 Quản Lý MCP Server Supervisor & Discovery** — discovery only; no supervisor/JSON-RPC client
  - File: `src/mcp/mod.rs`
  - Tự động đọc cấu hình MCP từ `~/.claude.json`, `~/.cursor/mcp.json`, Claude Desktop config, `.bencode/mcp.json` và `.mcp.json` (file > 2 MB bị bỏ qua và ghi log).
  - Chưa đọc Codex `~/.codex/config.toml` (không có dependency TOML trực tiếp).
  - TODO: supervisor khởi chạy/giám sát server và JSON-RPC client.
  - Phân tích transport (`stdio` / `sse`) và trạng thái enabled/disabled.
- [x] **4.2 Giao Diện Cấu Hình MCP**
  - File: `src/ui/settings_modal.rs` (Tab MCP)
  - Hiển thị danh sách các server MCP đang cấu hình, provider, scope và trạng thái kết nối.
- [ ] **4.3 Quick Composer (Global Floating Window)**
  - File: `src/quick_composer/mod.rs`, `src/quick_composer/window.rs`
  - Đăng ký global hotkey macOS (`Cmd+Shift+Space`).
  - Mở cửa sổ GPUI trong suốt nổi trên màn hình (spotlight-style).
  - Chụp màn hình vùng chọn (`screencapture -i`) đính kèm câu hỏi gửi nhanh cho Agent.

---

### 🔵 Phase 5: Multi-Agent Orchestration & Tích Hợp Thứ Ba (P2 - Điều Phối & Đám Mây)
*Mục tiêu: Làm việc nhóm giữa các Agent và liên kết công cụ quản lý dự án.*

- [ ] **5.1 Multi-Agent Orchestration (Đồ Thị Constellation)**
  - File: `src/ui/orchestration_view.rs`, `src/harness/orchestrator.rs`
  - Cho phép một agent chính chia nhỏ tác vụ và spawn các subagents chạy song song.
  - Vẽ đồ thị tương tác hiển thị trạng thái và tiến độ của từng subagent.
- [ ] **5.2 Tích Hợp Issue Trackers (GitHub, GitLab, Linear, Jira)**
  - File: `src/integrations/github.rs`, `src/integrations/linear.rs`, `src/integrations/jira.rs`
  - Kết nối API lấy danh sách Issue / Pull Request / Ticket.
  - Gõ lệnh `/issue <mã>` trong composer để tự động nhúng mô tả ticket vào ngữ cảnh của agent.
- [ ] **5.3 Remote Workspaces & SSH Host Engine**
  - File: `src/remote/mod.rs`, `src/remote/ssh.rs`
  - Kết nối tới server / máy ảo qua SSH (`~/.ssh/config`).
  - Đồng bộ file và chạy agent trực tiếp trên máy chủ từ xa.

---

### 🟣 Phase 6: System Integration, Menu Bar & Settings Persistence (P3 - Hoàn Thiện Tinh Gọn)
*Mục tiêu: Đóng gói thành ứng dụng macOS native hoàn chỉnh 100%.*

- [x] **6.1 macOS App Menu & Phím Tắt** (một phần)
  - File: `src/app/commands.rs` (GPUI `actions!` + `bind_keys` + `set_menus`)
  - Đã có: menu BenCode / File / View / Go và phím tắt theo MonoCode: `⌘,` `⌘K` `⌘T` `⌘W` `⌘S` `⌘B` `⌘J` `` ⌘` `` `⌘D` `⇧⌘D` `⌥⌘←→↑↓` `⇧⌘[` `⇧⌘]` `⌘Q`.
  - Còn thiếu: Edit menu (copy/paste qua menu), Window/Help, `⌘P` Go to File, `⇧⌘P` Command Palette, `⌘.` Switch Model.
- [ ] **6.2 System Tray & Dock Status Badge**
  - File: `src/macos/tray.rs`
  - Icon trên thanh Menu Bar hiển thị trạng thái Agent (Idle, Working, Waiting Approval).
  - Hiển thị badge số trên Dock icon khi có câu hỏi cần người dùng duyệt.
- [x] **6.3 Lưu Trữ Cấu Hình Settings** (một phần)
  - File: `src/settings.rs`, `src/app/preferences.rs`
  - MonoCode không lưu settings trong SQLite (DB không có bảng settings), nên BenCode dùng `~/Library/Application Support/BenCode/settings.json`, ghi nguyên tử ở nền, giữ nguyên key lạ.
  - Đang lưu: theme, model mặc định, chế độ quyền, trạng thái terminal. Chưa có: keybindings tuỳ biến, rate limits.
- [ ] **6.4 Project Wallpapers & Mascots**
  - File: `src/ui/project_theme.rs`, `src/ui/transcript.rs`
  - Cho phép cấu hình hình nền mờ (background wallpaper) và avatar đặc trưng riêng cho từng dự án.

---

## 📌 Bảng Đối Soát Trực Tiếp File: MonoCode -> BenCode

| Tính năng trong MonoCode | File Nguồn (MonoCode) | File Đích Cần Triển Khai (BenCode) | Trạng Thái |
| :--- | :--- | :--- | :--- |
| **Agent Execution Loop** | `integrations/harness/` + `src-tauri/src/harness.rs` | `src/harness/mod.rs`, `src/app.rs` | 🔨 Đang làm (Cần nối stream thật) |
| **Antigravity Harness** | `providers/antigravity/` | `src/harness/antigravity.rs` | ⏳ Chưa có |
| **Codex Harness** | `providers/codex/` | `src/harness/codex.rs` | ⏳ Chưa có |
| **OpenCode Harness** | `providers/opencode/` | `src/harness/opencode.rs` | ⏳ Chưa có |
| **Native Code Editor** | `@codemirror/*` (React) | `src/ui/editor_pane.rs` | 🟡 Partial (being fixed) |
| **MCP Supervisor** | `src-tauri/src/mcp.rs` | `src/mcp/mod.rs` | 🟡 Discovery only; no supervisor/JSON-RPC client |
| **Git Worktrees** | `src-tauri/src/worktrees.rs` | `src/git/worktrees.rs` | 🟡 Partial: engine ported, not wired into the app |
| **Checkpoints & Undo** | `src-tauri/src/checkpoint.rs` | `src/git/checkpoint.rs` | 🟡 Partial: engine ported, not wired into the app |
| **Quick Composer** | `src-tauri/src/quick_composer.rs` | `src/quick_composer/` | ⏳ Chưa có |
| **Orchestration View** | `features/orchestration/` | `src/ui/orchestration_view.rs` | ⏳ Chưa có |
| **Issue Trackers** | `src-tauri/src/github.rs, linear.rs` | `src/integrations/` | ⏳ Chưa có |
| **Remote SSH** | `src-tauri/src/remote_ssh.rs` | `src/remote/` | ⏳ Chưa có |
| **macOS Native Menu** | `src-tauri/src/menu.rs` | `src/app/commands.rs` | 🟡 Một phần (menu + phím tắt chính) |
| **System Tray** | `src-tauri/src/tray.rs` | `src/macos/tray.rs` | ⏳ Chưa có |
| **Settings Persistence** | webview storage | `src/settings.rs` (JSON) | 🟡 Một phần |
| **Split Chat Panels** | `features/workspace/ui/PaneTree.tsx` + `layout.ts` | `src/ui/layout.rs`, `src/ui/pane_tree.rs` | ✅ Hoàn thành |
| **Drag & Drop (Panes, Tabs, Files)** | `features/workspace/model/paneDrop.ts` | `src/ui/drag_drop.rs`, `src/ui/titlebar.rs`, `src/ui/file_tree.rs` | ✅ Hoàn thành |
| **Sidebar Toggle & Project Picker** | `features/workspace/ui/` | `src/ui/titlebar.rs`, `src/app.rs` | ✅ Hoàn thành |
| **Notes Scratchpad** | `features/notes/` + `notes.rs` | `src/ui/notes_view.rs` + `db/mod.rs` | ✅ Hoàn thành |
| **Automations (Cron)** | `features/automations/` | `src/ui/automations_view.rs` + `db/mod.rs` | ✅ Hoàn thành |
| **Diff Viewer** | `features/source-control/ui/Diff.tsx`| `src/ui/diff_viewer.rs` | ✅ Hoàn thành |
| **File Tree** | `features/files/ui/FileTree.tsx` | `src/ui/file_tree.rs` | ✅ Hoàn thành |
| **Terminal Pane** | `features/terminal/` | `src/ui/terminal_pane.rs` | 🟡 Có bản dựng (Cần multi-tab) |
