# BenCode ⚡

**BenCode** is a next-generation, 100% native GPU-accelerated Control Plane for AI Coding Agents (Claude Code, Codex, Antigravity, Cursor, Devin, Grok), written in **Rust** using **Zed's GPUI** and **[Ely GPUI Components](https://ely-gpui.zacharyzhang.com/)**.

---

## 🌟 Tầm nhìn & Mục tiêu của BenCode

1. **Hiệu năng cực hạn (Peak Performance)**:
   - 100% Native Rust, không Electron, không Chromium, không WebKit WebView.
   - Khởi động trong **20ms – 50ms**, ngốn chỉ **~30MB RAM**.
   - Render 120 FPS mượt mà bằng Apple Metal GPU Shaders.
2. **Quản lý Agent thông minh & Tiết kiệm Token**:
   - Tránh triệt để việc đốt token cho title generation hay context accumulation.
   - Giao tiếp trực tiếp với CLI qua stream JSON & stdio control channel.
3. **Git Worktree Isolation**:
   - Mỗi phiên làm việc của Agent tự động chạy trên một nhánh/worktree riêng ngầm, giữ sạch repo làm việc chính của lập trình viên.
4. **Local-first & Tương thích hệ sinh thái**:
   - Đọc và đồng bộ dữ liệu phiên làm việc linh hoạt, an toàn và bảo mật.

---

## 🛠 Kiến trúc hệ thống (Architecture)

```
bencode/
├── Cargo.toml         # Cấu hình GPUI, Ely Components, Tokio, Rusqlite
├── README.md          # Tài liệu dự án
└── src/
    ├── main.rs        # Entrypoint ứng dụng & Quản lý cửa sổ Native
    ├── app.rs         # State quản lý toàn bộ vòng đời ứng dụng
    ├── db/            # Tầng lưu trữ dữ liệu (SQLite, Session persistence)
    │   └── mod.rs
    ├── harness/       # Tầng giao tiếp với các Coding Agents (Claude, Codex, Antigravity)
    │   └── mod.rs
    └── ui/            # Tầng giao diện GPU (Sidebar, Transcript, Composer, Diffs)
        ├── mod.rs
        ├── sidebar.rs
        ├── transcript.rs
        └── composer.rs
```

---

## 🚀 Chạy thử nghiệm

```bash
cargo run
```
