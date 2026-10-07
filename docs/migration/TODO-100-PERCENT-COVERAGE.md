# BenCode: lộ trình đạt 100% tính năng của MonoCode

Tài liệu này ghi **những gì còn lại** để BenCode ngang bằng MonoCode, xếp theo
mức ưu tiên. Phần đã xong chỉ được tóm tắt.

- Trạng thái theo từng mảng: [`feature-migration-matrix.md`](feature-migration-matrix.md)
- Danh sách chi tiết từng mục: [`PARITY-BACKLOG.md`](PARITY-BACKLOG.md)

Đối chiếu với code lần cuối: 2026-10-06.

---

## 1. Đã hoàn thành

- **Agent**: chạy lượt thật qua stdio cho Claude Code, Antigravity, Codex,
  OpenCode; stream token và tool call; dừng lượt; hỏi quyền chạy tool; mỗi
  thread chạy độc lập, có hàng đợi tin nhắn; catalog model lấy trực tiếp từ CLI.
- **Chat**: transcript theo lượt, gập phần "đã làm việc", tìm trong hội thoại,
  thanh mục lục prompt ở mép phải (prompt outline);
  composer với model picker, chế độ quyền, `@` file, `/` skill, `/mcp`, đính kèm,
  handoff, câu hỏi làm rõ, thông báo giới hạn sử dụng, `/compact`.
- **Shell**: rail project (nhóm, ghim, menu, kéo thả, đổi kích thước), title
  bar với tab workspace, Back / Forward, sidebar Sessions (thư mục, bộ lọc,
  nhắc việc), chia pane và dock bằng kéo thả, menu bar và phím tắt chính.
- **File**: Explorer đầy đủ thao tác, trình soạn code native (lưu atomic, phát
  hiện file đổi trên đĩa), Go to File, mở bằng editor ngoài.
- **Git**: Changes panel (stage, unstage, discard, commit, amend, sync, PR), đồ
  thị commit, branch picker, tạo worktree khi gửi tin đầu tiên.
- **Review**: diff working tree và commit theo kiểu `UnifiedDiffView`, mở thành
  tab bên phải chat.
- **Session review**: card "Changed N files" sau mỗi lượt có sửa file, với
  Undo, Keep và Review thay đổi của riêng thread đó (dùng chung kho checkpoint
  với MonoCode).
- **Surface**: Search, Inbox GitHub (checks, comment, merge, sửa CI), Notes,
  Automations (có scheduler 30 giây), Settings.
- **Hạ tầng**: đọc ghi `monocode.db`, `settings.json`, terminal native theo
  project, phát hiện MCP server.

---

## 2. Việc còn lại

### P0 — Thiếu chức năng cốt lõi

- [~] **Vòng đời worktree**
  - Đã có: liệt kê, tạo worktree khi gửi tin đầu (`ui/composer/new_worktree.rs`);
    trang Settings › Worktrees (`ui/settings_worktrees.rs`) liệt kê worktree
    của một project, Reveal, và dialog "Delete worktree?" như MonoCode:
    luôn xoá ép buộc sau một lần xác nhận, tuỳ chọn "Also delete associated
    sessions"; thread được giữ lại bị tách khỏi worktree (`worktree_removed`)
    và chờ chọn working copy mới. Không xoá được worktree bị khoá, detached,
    hoặc đang có file, terminal, agent dùng.
    Trang có ô chọn project (đang mở, gần đây, đã lưu trữ) và dialog
    "Create worktree". Thread được tách qua journal `worktree_removals` của
    MonoCode trước khi git xoá: xoá lỗi thì khôi phục, bị gián đoạn thì xử lý
    ở lần mở database kế tiếp (`db/worktree_removals.rs`).
  - Cần: đổi tên nhánh theo tin nhắn đầu.
  - MonoCode: `source-control/ui/WorktreesPage.tsx`, `DeleteWorktreeDialog.tsx`,
    `src-tauri/src/worktrees.rs`, `worktree_lifecycle.rs`.
- [ ] **Cài đặt provider**: model mặc định theo provider, "Use by default",
  "Show in picker", ghi đè đường dẫn CLI.
- [ ] **Skills**: trang Skills trong Settings, bật tắt từng skill.
- [ ] **MCP**: thêm, xoá, đăng nhập, xem cấu hình; đọc cấu hình Codex
  (`~/.codex/config.toml`) và OpenCode. Hiện chỉ có phần phát hiện.

### P1 — Khác biệt thấy được

- [ ] **Review**
  - Tô màu cú pháp trong diff.
  - Chế độ diff hai cột trong editor, mặc định của MonoCode
    (`DIFF_VIEWER_DEFAULT = "editor"`).
- [ ] **Session review**: Keep / Undo cho từng file (engine đã hỗ trợ), tích
  hợp thay đổi của worker khi có orchestration (`session_checkpoint_apply`).
- [ ] **Pane file**: lưu theo từng tab workspace và khôi phục khi mở lại; chia
  pane editor; menu chuột phải trên tab.
- [ ] **Editor**: footer thay cho toolbar; xem trước ảnh và markdown.
- [ ] **Phím tắt và menu**: menu Edit / Window / Help, ⌘1-9, ⌃Tab, ⇧⌘A,
  trang Keybindings.
- [ ] **Search**: điều hướng bằng bàn phím, xếp hạng và phạm vi như MonoCode,
  mở file trong editor.
- [ ] **Notes**: chuyển Preview / Source, giao diện tag, lưu một lượt thành
  ghi chú, chèn `@note/`.
- [ ] **Automations**: trình sửa trigger, cài đặt session, bảng lịch sử chạy,
  Run now chạy song song.
- [ ] **Terminal**: link và tìm kiếm trong terminal; vị trí và kích thước dock.
- [ ] **Quick Composer**: cửa sổ nổi mở bằng phím tắt toàn hệ thống, đính kèm
  ảnh chụp màn hình. MonoCode: `features/quick-composer/`,
  `src-tauri/src/quick_composer.rs`.

### P2 — Mở rộng

- [ ] **Thêm harness**: Pi, OMP, Cursor, Grok, Hermes (MonoCode có trong
  `integrations/harness/providers/`; BenCode mới có icon).
- [ ] **OpenCode rewind**: cần chạy qua `opencode serve` thay cho `opencode run`.
- [ ] **Orchestration nhiều agent**: BenCode chỉ đọc bản ghi của MonoCode để
  hiện badge trên card; chưa có orchestrator và đồ thị subagent.
  MonoCode: `features/orchestration/`.
- [ ] **Issue tracker khác GitHub**: GitLab, Linear, Jira, Azure DevOps.
  MonoCode: `src-tauri/src/{gitlab,linear,jira,azure_devops}.rs`.
- [ ] **Remote workspace qua SSH**. MonoCode: `src-tauri/src/remote_ssh.rs`,
  `remote.rs`.
- [ ] **MCP supervisor**: khởi chạy, giám sát server và JSON-RPC client.
- [ ] **Tích hợp macOS**: icon trên menu bar, badge trên Dock, thông báo hệ
  thống. MonoCode: `src-tauri/src/tray.rs`, `notifications.rs`.
- [ ] **Giao diện**: hình nền chat theo project (`chat_background.rs`), panel
  "Working agents", thông báo cập nhật harness (`harness_updates.rs`).

---

## 3. Bảng đối chiếu file cho phần chưa xong

| Tính năng | MonoCode | BenCode | Trạng thái |
| :--- | :--- | :--- | :--- |
| Vòng đời worktree | `src-tauri/src/worktree_lifecycle.rs` | `src/git/worktrees.rs`, `src/app/worktree_lifecycle.rs`, `src/ui/settings_worktrees.rs` | 🟡 Thiếu đổi tên nhánh |
| MCP | `src-tauri/src/mcp.rs` | `src/mcp/mod.rs` | 🟡 Chỉ phát hiện |
| Menu macOS | `src-tauri/src/menu.rs` | `src/app/commands.rs` | 🟡 Thiếu Edit / Window / Help |
| Review hai cột | `@codemirror/merge` | `src/ui/diff_viewer.rs` | 🟡 Chỉ có unified |
| Orchestration | `features/orchestration/` | `src/db/orchestration.rs` | 🟡 Chỉ đọc |
| Quick Composer | `src-tauri/src/quick_composer.rs` | chưa có | ⚪ |
| Tracker khác | `src-tauri/src/{gitlab,linear,jira,azure_devops}.rs` | chưa có | ⚪ |
| Remote SSH | `src-tauri/src/remote_ssh.rs` | chưa có | ⚪ |
| Tray, Dock badge | `src-tauri/src/tray.rs` | chưa có | ⚪ |
| Hình nền chat | `src-tauri/src/chat_background.rs` | chưa có | ⚪ |

---

## 4. Cách làm một mục

1. Đọc model và component tương ứng trong `reference/monocode`.
2. Port phần logic thành hàm thuần có test trước, rồi mới dựng view.
3. Tuân theo quy tắc trong [`AGENTS.md`](../../AGENTS.md).
4. Chạy `cargo test`, chạy app và thử trực tiếp.
5. Đánh dấu mục ở đây và trong `PARITY-BACKLOG.md`, cập nhật trạng thái trong
   `feature-migration-matrix.md`.
