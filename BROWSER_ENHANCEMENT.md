# TiDev 浏览器增强方案

本文档探讨使用 [rust-headless-chrome](https://github.com/rust-headless-chrome/rust-headless-chrome) 增强 TiDev 网页抓取和 Web 调试能力的可行性方案。

## 背景

### 现有 webfetch 工具的局限

TiDev 当前的 `webfetch` 工具（`src/tooling/builtin/web.rs`）基于 `reqwest` HTTP 客户端实现，存在以下局限：

1. **无法处理 JavaScript 渲染页面**：SPA（单页应用）如 React/Vue/Angular 应用，内容通过 JS 动态加载，webfetch 只能获取初始 HTML
2. **无法处理需要交互的场景**：登录后的内容、点击"加载更多"、滚动加载等
3. **无法获取视觉信息**：没有截图能力，无法查看页面实际渲染效果
4. **反爬虫限制**：某些网站通过 Cloudflare 等保护，纯 HTTP 请求容易被拦截

### rust-headless-chrome 能力

[rust-headless-chrome](https://github.com/rust-headless-chrome/rust-headless-chrome) 是 Puppeteer 的 Rust 等价物，通过 Chrome DevTools Protocol 控制浏览器：

| 功能 | 描述 |
|------|------|
| 页面导航 | `navigate_to()`, `wait_for_element()` |
| 元素交互 | `click()`, `type_str()`, `press_key()` |
| JavaScript 执行 | `call_js_fn()`, `evaluate_script()` |
| 截图 | `capture_screenshot()` 支持全页/视口/元素 |
| PDF 生成 | `print_to_pdf()` |
| 网络拦截 | `enable_request_interception()` |
| JS 覆盖率 | `take_precise_js_coverage()` |
| 隐身模式 | 支持创建隐身窗口 |
| 扩展加载 | 支持预加载 Chrome 扩展 |

## 方案一：浏览器抓取工具

### 概述

新增 `browser_fetch` 工具，使用 headless Chrome 渲染页面后提取内容。

### 工具定义

```rust
tool_args! {
    pub struct BrowserFetchArgs {
        url: string("The URL to fetch"),
        wait_for: optional_string("CSS selector to wait for before extracting content"),
        format: optional_string("Output format: markdown (default), html, text"),
        timeout: optional_number("Timeout in seconds (default 60, max 120)"),
        execute_js: optional_string("JavaScript to execute before extraction"),
    }
}
```

### 使用场景

```
// 抓取 React SPA 页面，等待内容加载
browser_fetch(url="https://example.com/app", wait_for=".content-loaded")

// 执行 JS 后抓取（如展开折叠内容）
browser_fetch(url="https://example.com/article", execute_js="document.querySelectorAll('.collapsed').forEach(e => e.click())")
```

### 实现要点

1. **浏览器实例管理**：
   - 使用 `Browser::default()` 创建实例
   - 考虑连接到已有浏览器实例（`Browser::connect()`）以复用进程
   - 或使用 `--remote-debugging-port` 连接到用户正在使用的 Chrome

2. **内容提取**：
   ```rust
   // 等待页面加载
   tab.navigate_to(url)?;
   if let Some(selector) = wait_for {
       tab.wait_for_element(selector)?;
   }
   // 执行额外 JS
   if let Some(js) = execute_js {
       tab.evaluate_script(js, false)?;
   }
   // 提取内容
   let html = tab.get_content()?;  // 或执行 JS 获取 innerHTML
   // 转换为 markdown
   let markdown = html2md::convert(&html);
   ```

3. **超时处理**：使用 `tab.wait_for_element_with_timeout()`

### 优缺点

| 优点 | 缺点 |
|------|------|
| 解决 JS 渲染问题 | 需要安装 Chrome/Chromium |
| 更接近真实浏览器行为 | 内存占用较高 |
| 可处理复杂交互 | 启动速度比 HTTP 请求慢 |

## 方案二：截图工具

### 概述

新增 `browser_screenshot` 工具，对网页进行截图。

### 工具定义

```rust
tool_args! {
    pub struct BrowserScreenshotArgs {
        url: string("The URL to screenshot"),
        selector: optional_string("CSS selector for element screenshot (default: full page)"),
        full_page: optional_bool("Capture full scrollable page (default: true)"),
        format: optional_string("Image format: png (default), jpeg"),
        viewport_width: optional_number("Viewport width in pixels (default: 1280)"),
        viewport_height: optional_number("Viewport height in pixels (default: 800)"),
        wait_for: optional_string("CSS selector to wait for before screenshot"),
        timeout: optional_number("Timeout in seconds (default 60)"),
    }
}
```

### 使用场景

```
// 全页截图
browser_screenshot(url="https://example.com")

// 元素截图
browser_screenshot(url="https://example.com", selector=".chart-container")

// 移动端视口
browser_screenshot(url="https://example.com", viewport_width=375, viewport_height=667)
```

### 实现要点

```rust
use headless_chrome::protocol::cdp::Page;

let browser = Browser::default()?;
let tab = browser.new_tab()?;

// 设置视口
tab.set_default_viewport(Viewport {
    width: viewport_width,
    height: viewport_height,
    ..Default::default()
})?;

tab.navigate_to(url)?;
if let Some(selector) = wait_for {
    tab.wait_for_element(selector)?;
}

let format = match format.as_str() {
    "jpeg" => Page::CaptureScreenshotFormatOption::Jpeg,
    _ => Page::CaptureScreenshotFormatOption::Png,
};

let image_data = if let Some(sel) = selector {
    // 元素截图
    tab.wait_for_element(sel)?.capture_screenshot(format)?
} else {
    // 全页截图
    tab.capture_screenshot(format, None, None, full_page)?
};

// 返回 base64 编码或保存到临时文件
```

### 输出方式

1. **Base64 嵌入**：直接返回 base64 字符串（适合小截图）
2. **临时文件**：保存到临时文件，返回路径（适合大截图）
3. **返回给 LLM**：如果模型支持图像，直接返回图像数据

## 方案三：Web 调试 Skill

### 概述

创建一个 skill 文档，指导 LLM 如何使用浏览器工具调试 Web 应用。

### Skill 定义

位置：`~/.config/tidev/skills/web-debug/SKILL.md`

```markdown
---
name: web-debug
description: Debug and analyze web applications with browser tools
---

# Web Debugging Skill

This skill helps you debug and analyze web applications using browser automation tools.

## Available Tools

### browser_fetch
Fetch and render JavaScript-heavy pages. Use this when:
- The page content is rendered by JavaScript
- You need to wait for specific elements to load
- You need to execute JavaScript before extraction

### browser_screenshot
Capture visual snapshots of web pages. Use this when:
- You need to see the visual appearance of a page
- You want to capture specific UI elements
- You're debugging layout or rendering issues

## Common Workflows

### Debug a SPA Page
1. First screenshot to see current state: `browser_screenshot(url="...")`
2. Fetch rendered content: `browser_fetch(url="...", wait_for=".app-ready")`
3. If needed, execute custom JS: `browser_fetch(url="...", execute_js="...")`

### Debug Network Issues
1. Use browser_fetch with extended timeout
2. Check if content is behind authentication
3. Screenshot to verify Cloudflare or other challenges

### Debug Responsive Design
1. Take screenshots at different viewports
2. Compare mobile vs desktop rendering

## Tips

- Always specify `wait_for` when content loads asynchronously
- Use `execute_js` to interact with the page before extraction
- For authentication-required pages, consider connecting to user's existing Chrome instance
```

### 优缺点

| 优点 | 缺点 |
|------|------|
| 无需代码修改 | 依赖其他工具实现 |
| 纯文档形式，易于维护 | 功能受限于已有工具 |

## 方案四：完整浏览器工具集

### 概述

实现一组完整的浏览器自动化工具，覆盖导航、交互、执行、检查等场景。

### 工具列表

| 工具 | 描述 | 权限 |
|------|------|------|
| `browser_navigate` | 导航到 URL | Read |
| `browser_click` | 点击元素 | Execute |
| `browser_type` | 输入文本 | Execute |
| `browser_screenshot` | 截图 | Read |
| `browser_evaluate` | 执行 JavaScript | Execute |
| `browser_get_content` | 获取页面 HTML | Read |
| `browser_get_console` | 获取控制台日志 | Read |
| `browser_get_network` | 获取网络请求 | Read |
| `browser_close` | 关闭浏览器 | Session |

### 会话状态管理

需要引入浏览器会话管理：

```rust
// 在 app state 中维护浏览器实例
struct BrowserSession {
    browser: Option<Browser>,
    current_tab: Option<Arc<Tab>>,
    console_logs: Vec<ConsoleLog>,
    network_requests: Vec<NetworkRequest>,
}
```

### 使用场景

```
// 完整的登录流程
browser_navigate(url="https://example.com/login")
browser_type(selector="#username", text="user@example.com")
browser_type(selector="#password", text="password")
browser_click(selector="button[type=submit]")
browser_screenshot()
browser_get_content()
```

### 实现复杂度

1. **会话生命周期**：需要管理浏览器实例的创建、复用、销毁
2. **错误恢复**：处理浏览器崩溃、超时等异常
3. **资源管理**：限制内存占用，及时清理
4. **并发控制**：多标签页管理

## 技术实现考量

### 依赖引入

```toml
# Cargo.toml
[dependencies]
headless_chrome = { version = "1.0", features = ["fetch"] }
```

`fetch` feature 可自动下载 Chrome 二进制，无需用户手动安装。

### Chrome 二进制管理

| 方式 | 描述 | 适用场景 |
|------|------|---------|
| 自动下载 | 使用 `fetch` feature 自动下载 | 用户无 Chrome |
| 系统 Chrome | 使用用户安装的 Chrome | 用户有 Chrome |
| 远程调试 | 连接到运行中的 Chrome | 调试用户当前页面 |

```rust
// 自动下载
let browser = Browser::new(LaunchOptionsBuilder::default().build().unwrap())?;

// 使用系统 Chrome
let browser = Browser::new(
    LaunchOptionsBuilder::default()
        .path(Some(PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")))
        .build()
        .unwrap()
)?;

// 连接远程调试
let browser = Browser::connect("127.0.0.1:9222".parse().unwrap())?;
```

### 异步适配

`headless_chrome` 是同步 API，但 TiDev 使用异步模型（tokio）：

```rust
// 方案 1: 使用 spawn_blocking
let result = tokio::task::spawn_blocking(move || {
    // headless chrome 操作
}).await?;

// 方案 2: 使用独立线程池
static BROWSER_POOL: Lazy<ThreadPool> = Lazy::new(|| {
    ThreadPoolBuilder::new().num_threads(2).build().unwrap()
});
```

### 配置项

建议在 `config.toml` 中添加浏览器配置：

```toml
[browser]
# Chrome 可执行文件路径，为空则自动检测/下载
chrome_path = ""
# 是否启用浏览器功能
enabled = true
# 默认视口宽度
viewport_width = 1280
# 默认视口高度
viewport_height = 800
# 截图格式
screenshot_format = "png"
# 是否启用 fetch feature（自动下载 Chrome）
auto_fetch_chrome = true
```

### 权限控制

浏览器工具涉及敏感操作，需要合理设置权限：

| 操作类型 | 权限级别 | 说明 |
|---------|---------|------|
| 读取页面内容 | Read | 安全，只读操作 |
| 截图 | Read | 安全，只读操作 |
| 导航 | Execute | 可能触发服务器端操作 |
| 点击/输入 | Execute | 可能修改数据 |
| 执行 JS | Execute | 高风险，可能修改数据 |
| 关闭浏览器 | Session | 会话级操作 |

## 推荐实现路径

### Phase 1: 基础功能（建议首先实现）

1. **添加依赖**：引入 `headless_chrome` crate
2. **实现 `browser_screenshot`**：最简单，价值明确
3. **实现 `browser_fetch`**：解决 JS 渲染问题
4. **添加配置支持**：Chrome 路径、视口等

### Phase 2: 增强功能

1. **实现 `browser_evaluate`**：执行自定义 JS
2. **实现 `browser_get_console`**：获取控制台日志
3. **创建 `web-debug` skill**：提供使用指南

### Phase 3: 完整工具集（可选）

1. 实现交互工具（click, type, navigate）
2. 实现会话管理
3. 实现网络监控

## 风险与挑战

| 风险 | 缓解措施 |
|------|---------|
| Chrome 二进制依赖 | 提供 `fetch` feature 自动下载 |
| 内存占用高 | 限制浏览器实例数量，及时关闭 |
| 启动速度慢 | 复用浏览器实例，预热 |
| 与现有 webfetch 功能重叠 | 明确区分使用场景，webfetch 用于简单请求 |
| 反爬虫检测 | 使用真实浏览器行为，可配置 user agent |
| 跨平台兼容性 | 测试 Linux/macOS/Windows |

## 参考资料

- [rust-headless-chrome 文档](https://docs.rs/headless_chrome)
- [rust-headless-chrome 示例](https://github.com/rust-headless-chrome/rust-headless-chrome/tree/main/examples)
- [Chrome DevTools Protocol](https://chromedevtools.github.io/devtools-protocol/)
- [Puppeteer 文档](https://pptr.dev/)（API 设计参考）
