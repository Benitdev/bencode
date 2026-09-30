# BenCode: Kế Hoạch & TODO List Hoàn Thiện 100% Tính Năng Từ MonoCode ⚡

> **Mục tiêu**: Chuyển đổi toàn diện MonoCode (Tauri v2 + React 19 + TypeScript) sang BenCode (100% Native Rust + Zed GPUI + Ely GPUI Components), đạt hiệu năng cực hạn (GPU Metal 120 FPS, khởi động <50ms, RAM ~30MB) và tương thích hoàn toàn hệ sinh thái dữ liệu của MonoCode.

---

## 📊 Tổng Quan Hiện Trạng (Audit Summary)

- **Đã hoàn thành sơ bộ (~45%)**:
  - Khung kiến trúc GPUI 100% Native Rust, tích hợp thư viện [Ely GPUI Components](https://ely-gpui.zacharyzhang.com/).
  - Hệ thống giao diện: Left Rail, Sidebar 3 chế độ (Sessions, File Tree, Git Changes), Titlebar, Transcript Timeline, Prompt Composer (kèm Model/Branch/Skill/Mention picker), Diff Viewer, Terminal Pane, Settings Modal, Notes View, Automations View, Search Modal, Footer.
  - Tầng dữ liệu SQLite: Kết nối trực tiếp với DB của MonoCode (`monocode.db`), tương thích schema sessions, notes, automations.
  - Git CLI operations: Lấy status, branch, diff, commits, staging.
  - Claude CLI harness prototype: Parser streaming JSON.

- **Chưa hoàn thiện / Cần triển khai (~55%)**:
  - Kết nối luồng thực thi thật (Live Stdio Streaming) từ Composer vào Agent Process (thay cho mock response hiện tại).
  - Bổ sung các harness provider khác: Antigravity, Codex, OpenCode, Cursor, Grok, Pi.
  - Git Worktree isolation (mỗi session chạy trên 1 worktree ngầm) & Checkpoint/Undo turn.
  - Trình xem/chỉnh sửa code native (Native Code Editor) thay vì chỉ xem cây thư mục.
  - Hệ thống MCP (Model Context Protocol) client supervisor & picker.
  - Quick Composer (Cửa sổ floating native mở bằng phím tắt toàn hệ thống).
  - Multi-Agent Orchestration (Đồ thị Constellation quản lý subagents).
  - Tích hợp Issue Tracker (GitHub, GitLab, Linear, Jira) & Remote SSH workspaces.
  - macOS Menu bar, System Tray, Dock Badge, và lưu trữ Settings vào SQLite.

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

### 🟠 Phase 2: Git Worktree Isolation & Checkpoint Snapshot (P1 - An Toàn Mã Nguồn)
*Mục tiêu: Bảo vệ workspace của lập trình viên, cho phép rollback khi agent làm sai.*

- [ ] **2.1 Worktree Session Isolation**
  - File: `src/git/worktrees.rs`, `src/app.rs`
  - Tự động tạo git worktree ngầm tại `.bencode/worktrees/<session-id>` khi bắt đầu phiên làm việc mới.
  - Trỏ `cwd` của agent vào worktree này để không làm bẩn working directory chính của lập trình viên.
  - Cung cấp thao tác "Merge to Main" hoặc "Discard Branch" khi phiên kết thúc.
- [ ] **2.2 Checkpoint & Undo Turn Engine**
  - File: `src/git/checkpoint.rs`, `src/ui/transcript.rs`
  - Tự động tạo git stash/commit checkpoint trước mỗi turn người dùng nhập prompt.
  - Nút "Restore to this point" trên mỗi block trong transcript để khôi phục mã nguồn về trạng thái trước đó.

---

### 🟡 Phase 3: Native Editor & Nâng Cấp UI/UX (P1 - Trải Nghiệm Lập Trình)
*Mục tiêu: Trải nghiệm xem/sửa code, terminal và composer hoàn chỉnh.*

- [ ] **3.1 Trình Xem & Biên Tập Code Native (Native Code Editor)**
  - File: `src/ui/editor_pane.rs`, `src/workspace.rs`
  - Tích hợp `ely_gpui_component::editor` để mở và chỉnh sửa file khi bấm vào file tree.
  - Hỗ trợ cú pháp highlight (Rust, TypeScript, Python, JSON, Markdown).
  - Phím tắt lưu file (`Cmd+S`), phát hiện định dạng dòng LF/CRLF.
- [ ] **3.2 Mở File Bằng Trình Soạn Thảo Ngoài (External Editor)**
  - File: `src/external_editor.rs`, `src/ui/file_tree.rs`
  - Thêm menu chuột phải: "Open in VS Code" (`code`), "Open in Cursor" (`cursor`), "Open in Zed" (`zed`).
- [ ] **3.3 Image Paste & Đính Kèm File Trong Composer**
  - File: `src/ui/composer.rs`, `src/pasteboard.rs`
  - Bắt sự kiện dán ảnh từ clipboard (`Cmd+V`), lưu vào cache ảnh tạm và hiển thị thumbnail preview trong composer.
- [ ] **3.4 Nâng Cấp Terminal Pane**
  - File: `src/ui/terminal_pane.rs`
  - Hỗ trợ mở nhiều tab terminal, lựa chọn shell (Zsh, Bash, Fish), truyền đúng biến môi trường dự án.

---

### 🟢 Phase 4: MCP (Model Context Protocol) & Quick Composer (P2 - Tính Năng Cao Cấp)
*Mục tiêu: Mở rộng khả năng của Agent qua MCP và phím tắt toàn hệ thống.*

- [ ] **4.1 Quản Lý MCP Server Supervisor**
  - File: `src/mcp/mod.rs`, `src/mcp/client.rs`
  - Tự động đọc cấu hình MCP từ `~/.claude/mcp.json`, `~/.cursor/mcp.json` và `.bencode/mcp.json`.
  - Khởi chạy và giám sát tiến trình MCP stdio server.
  - Giao tiếp JSON-RPC 2.0 (khám phá `tools/list`, gọi `tools/call`).
- [ ] **4.2 Giao Diện Cấu Hình MCP**
  - File: `src/ui/settings_modal.rs` (Tab MCP), `src/ui/composer.rs`
  - Bật/tắt từng server MCP, xem danh sách công cụ đang hoạt động.
  - Gợi ý slash command `/mcp` trong composer.
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

- [ ] **6.1 macOS App Menu & Phím Tắt Toàn Diện**
  - File: `src/macos/menu.rs`, `src/main.rs`
  - Menu Bar chuẩn macOS: File, Edit, View, Window, Help với đầy đủ accelerators (`Cmd+N`, `Cmd+W`, `Cmd+F`, `Cmd+,`).
- [ ] **6.2 System Tray & Dock Status Badge**
  - File: `src/macos/tray.rs`
  - Icon trên thanh Menu Bar hiển thị trạng thái Agent (Idle, Working, Waiting Approval).
  - Hiển thị badge số trên Dock icon khi có câu hỏi cần người dùng duyệt.
- [ ] **6.3 Lưu Trữ Cấu Hình Settings Vào SQLite**
  - File: `src/ui/settings_modal.rs`, `src/db/mod.rs`
  - Bảng `settings` trong SQLite lưu: Theme, Default Harness, API Keys, Keybindings, Rate limits.
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
| **Git Worktrees** | `src-tauri/src/worktrees.rs` | `src/git/worktrees.rs` | ⏳ Chưa có |
| **Checkpoints & Undo** | `src-tauri/src/checkpoint.rs` | `src/git/checkpoint.rs` | ⏳ Chưa có |
| **Native Code Editor** | `@codemirror/*` (React) | `src/ui/editor_pane.rs` | ⏳ Chưa có (Dùng Ely editor) |
| **MCP Supervisor** | `src-tauri/src/mcp.rs` | `src/mcp/mod.rs` | ⏳ Chưa có |
| **Quick Composer** | `src-tauri/src/quick_composer.rs` | `src/quick_composer/` | ⏳ Chưa có |
| **Orchestration View** | `features/orchestration/` | `src/ui/orchestration_view.rs` | ⏳ Chưa có |
| **Issue Trackers** | `src-tauri/src/github.rs, linear.rs` | `src/integrations/` | ⏳ Chưa có |
| **Remote SSH** | `src-tauri/src/remote_ssh.rs` | `src/remote/` | ⏳ Chưa có |
| **macOS Native Menu** | `src-tauri/src/menu.rs` | `src/macos/menu.rs` | ⏳ Chưa có |
| **System Tray** | `src-tauri/src/tray.rs` | `src/macos/tray.rs` | ⏳ Chưa có |
| **Settings DB Save** | `src-tauri/src/session_store.rs` | `src/db/mod.rs` (bảng settings) | ⏳ Chưa có |
| **Notes Scratchpad** | `features/notes/` + `notes.rs` | `src/ui/notes_view.rs` + `db/mod.rs` | ✅ Hoàn thành |
| **Automations (Cron)** | `features/automations/` | `src/ui/automations_view.rs` + `db/mod.rs` | ✅ Hoàn thành |
| **Diff Viewer** | `features/source-control/ui/Diff.tsx`| `src/ui/diff_viewer.rs` | 🟡 Có bản dựng (Cần tối ưu) |
| **File Tree** | `features/files/ui/FileTree.tsx` | `src/ui/file_tree.rs` | 🟡 Có bản dựng (Cần tích hợp mở tab) |
| **Terminal Pane** | `features/terminal/` | `src/ui/terminal_pane.rs` | 🟡 Có bản dựng (Cần multi-tab) |
