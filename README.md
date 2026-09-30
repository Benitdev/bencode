# MonoCode GPUI (Proof of Concept)

A GPU-accelerated, 100% native Rust control plane for **MonoCode** built with **Zed's GPUI** and **[Ely GPUI Components](https://ely-gpui.zacharyzhang.com/)**.

---

## 🚀 Tính năng của bản PoC (Phase 1)

1. **100% Native Rust & GPUI**:
   - Khởi động tức thì (~20-50ms), tiêu thụ cực ít RAM (~30-40MB).
   - Render trực tiếp bằng Apple Metal GPU Shaders, tần số quét 120 FPS mượt mà.
   - Hoàn toàn không phụ thuộc vào WebKit, Chromium hay JavaScript Runtime.
2. **Tích hợp Ely GPUI Components**:
   - Tận dụng hệ thống component tinh gọn của `ely-gpui-component` (Theme, Palettes, Layouts, Scrollbars).
   - Dark Mode chuẩn mực với palette màu dịu mắt (`Mode::Dark`).
3. **Đọc trực tiếp dữ liệu từ MonoCode**:
   - Kết nối trực tiếp vào file SQLite database thật của MonoCode tại:
     `~/Library/Application Support/com.monocode.desktop/monocode.db`.
   - Tự động nạp danh sách các phiên làm việc gần nhất (Sessions), tên nhánh Git (`branch`), thư mục dự án (`cwd`), và loại Agent (`Claude`, `Codex`, `Antigravity`).
4. **Sidebar & Workspace Interaction**:
   - Cho phép click chọn từng session trong sidebar để chuyển đổi trực tiếp trên giao diện native.
   - Hiển thị thông tin phiên, Agent Model và khung Composer dock ở chân trang.

---

## 🛠 Cách chạy thử nghiệm

Mở Terminal tại thư mục này và gõ:

```bash
cargo run
```

---

## 🗺 Lộ trình phát triển tiếp theo (Roadmap)

- [x] **Phase 1 (Scaffold & PoC)**: Cửa sổ GPUI native, theme Ely, đọc database SQLite thực tế của MonoCode.
- [ ] **Phase 2 (Transcript Streaming)**: Hiển thị các khối tin nhắn (Blocks) từ `blocks_json` với Markdown parsing và code syntax highlight.
- [ ] **Phase 3 (Agent Execution)**: Gọi trực tiếp `claude` CLI / `codex` / `antigravity` qua `tokio::process::Command` và stream kết quả thời gian thực vào transcript.
- [ ] **Phase 4 (Diff Viewer & Git Worktree)**: Sử dụng component `changes` và `similar` từ Ely để review git diff theo từng hunk.
- [ ] **Phase 5 (Embedded Terminal)**: Nhúng `alacritty_terminal` native để mở terminal session song song.
