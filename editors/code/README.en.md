# TyranoScript for VS Code

<p>
  <a href="https://marketplace.visualstudio.com/items?itemName=darallium.tyranoscript-language-server"><img src="https://img.shields.io/visual-studio-marketplace/v/darallium.tyranoscript-language-server?style=flat-square&label=Marketplace&color=6f42c1" alt="VS Code Marketplace Version"></a>
  <a href="https://marketplace.visualstudio.com/items?itemName=darallium.tyranoscript-language-server"><img src="https://img.shields.io/visual-studio-marketplace/i/darallium.tyranoscript-language-server?style=flat-square&label=Installs&color=0078d7" alt="Installs"></a>
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="License: MIT">
</p>

**🌐 言語 / Language:** [日本語](README.md) ・ **English** ・ [中文](README.cn.md)

Language support for the visual-novel engine **[TyranoScript](https://tyrano.jp/)** (`.ks` scenario files) in VS Code.
On top of syntax highlighting, the Rust-based language server **`tyrano-lsp`** brings full "IDE" features — diagnostics, completion, go-to-definition and find-references — to your `.ks` files.

![A TyranoScript scenario file opened in VS Code](images/highlighting.png)

---

## 🎬 Who is this for?

- You want to catch **misspelled tags** and broken `storage=` / label targets **before you save**.
- You edit scenarios spread across **many `.ks` files** and want to follow labels and macros between them.
- You want to type tag names and parameters **from completion** instead of memorizing them.

---

## ✨ Features

| Feature | What it does | Server |
|---------|--------------|:---:|
| [Syntax highlighting](#-syntax-highlighting) | Colors tags, labels, comments and embedded JS/HTML | Not required |
| [Real-time diagnostics](#-real-time-diagnostics) | Flags unknown tags, unresolved jumps, missing assets | Required |
| [Hover information](#-hover-information) | Pops up docs for tags, parameters and labels | Required |
| [Completion](#-completion) | Suggests tag names, parameter names and values | Required |
| [Go to definition / references](#-go-to-definition--references) | Tracks labels and macros across files | Required |
| [Outline / breadcrumbs](#-outline--breadcrumbs) | Navigable list of labels, macros and characters | Required |

> Features marked "Required" use the `tyrano-lsp` language server — see [Setup](#-setup). **Syntax highlighting works with the extension alone**, no server needed.

---

### 🎨 Syntax highlighting

Opening a `.ks` file colorizes every element via a TextMate grammar dedicated to TyranoScript.
It is available **from the extension alone**, without waiting for the language server to start, so scenarios are readable the moment you open them.

![Syntax highlighting example](images/highlighting.png)

Highlighted elements include:

- **Label definitions** — `*start|オープニング` (the caption after `|` is distinguished too)
- **Tags** — `[bg storage=room.jpg time=1000]` (tag name, parameter names and values each colored)
- **Line comments** — `; this is a comment`
- **Character-name lines** — `#akane` / `#akane:happy`
- **Embedded scripts** — content inside `[iscript]` … `[endscript]` is highlighted **as JavaScript**, and `[html]` … `[endhtml]` **as HTML**

---

### 🚦 Real-time diagnostics

Every edit and save re-analyzes the whole scenario and underlines problems with **squiggles**.
Analysis spans the **entire project (multiple files)**, so labels in other files and assets under `data/` are validated too — not just the current file.

![Diagnostics and the Problems panel](images/diagnostics.png)

Examples of what it detects:

| Diagnostic code | Meaning |
|-----------------|---------|
| `xsem-unknown-tag` | An **unknown tag** that is neither a builtin nor a visible macro (e.g. a typo like `[teleprot]`) |
| `sem-unknown-label-target` | A **jump to a label** that does not exist in the same file (`target=*nowhere`) |
| `xsem-unknown-label-in-storage` | A `[jump]` to a **label missing from another file** (`storage=scene2.ks target=*missing`) |
| `xsem-missing-asset` | An **image/audio asset** referenced by `storage=` cannot be found |
| `xsem-unknown-param` / `xsem-missing-param` | An **unknown parameter**, or a **missing required parameter**, on a tag |

All problems appear in the **Problems** panel (`Ctrl+Shift+M`); clicking one jumps to the location.

---

### 💡 Hover information

Hover the mouse over a tag, parameter or label (or place the cursor and press `Ctrl+K Ctrl+I`) to see a documentation popup.
In particular, hovering a `[jump]` target label resolves **which file the label is defined in**.

![Hovering a jump target label](images/hover.png)

In the example above, hovering `target=*top` reveals at a glance that `*top` is a label defined in `data/scenario/scene2.ks`.

---

### ⌨️ Completion

Tag names, parameter names and parameter values are suggested automatically.
Right after typing `[`, or by pressing `Ctrl+Space` mid-word, a candidate list opens — with **per-item documentation** shown on the side.

![Tag-name completion](images/completion.png)

- **Tag-name completion** — typing `[cha` suggests `chara_show` / `chara_new` / `chara_hide` …
- **Parameter-name completion** — only the parameters that tag accepts
- **Project macros are candidates too** — a macro defined with `[macro name=greet]` completes as `[greet]`

---

### 🔗 Go to definition / references

On any label or macro you can use **Go to Definition** (`F12`), **Peek** (`Alt+F12`) and **Find All References** (`Shift+F12`).
Jumps that point to another file via `storage=` are resolved, so you can **follow the scenario flow across files**.

![Peeking a label definition in another file](images/definition.png)

Above, we follow `*top` from `[jump storage=scene2.ks target=*top]` in `first.ks` and see the defining line in `scene2.ks` expanded inline.

---

### 🗂️ Outline / breadcrumbs

The **labels, macros and characters** in a file are structured and listed in the sidebar's Outline view and in the editor's breadcrumbs.
Even in long scenarios, you can jump to any label with a single click.

![Outline view showing labels, macros and characters](images/outline.png)

Press `Ctrl+Shift+O` to open symbol search and jump quickly by typing a label name.

---

## 🚀 Setup

### 1. Install the extension

Search for **"TyranoScript"** in the Extensions view (`Ctrl+Shift+X`), or install from the [Marketplace page](https://marketplace.visualstudio.com/items?itemName=darallium.tyranoscript-language-server).

Opening a `.ks` file activates it automatically, and **syntax highlighting is available at that point**.

### 2. Provide the `tyrano-lsp` language server (needed for diagnostics, completion, etc.)

Diagnostics, hover, completion and go-to-definition require the Rust-based language server **`tyrano-lsp`**. The extension searches for it in this order:

1. The absolute path in the `tyranoscript.server.path` setting
2. `tyrano-lsp` on your `PATH`
3. `<extension>/server/tyrano-lsp`
4. `<workspace>/target/release/tyrano-lsp`
5. `<workspace>/target/debug/tyrano-lsp`

To build from source, clone the [repository](https://github.com/darallium/tyrano-parser) and run:

```bash
cargo build --release -p tyrano-lsp
```

Then place the resulting `target/release/tyrano-lsp` in one of the locations above, or point to it via settings:

```jsonc
// settings.json
{
  "tyranoscript.server.path": "/absolute/path/to/tyrano-lsp"
}
```

> If the server is not found, the extension shows an error message explaining how to install it. After setting the path, restart the server via **Command Palette (`Ctrl+Shift+P`) → "TyranoScript: Restart Language Server"**.

---

## ⚙️ Settings & commands

### Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `tyranoscript.server.path` | `""` | Absolute path to the `tyrano-lsp` executable. When empty, it is auto-detected using the search order above. |
| `tyranoscript.trace.server` | `off` | Communication log level between VS Code and the server (`off` / `messages` / `verbose`). Handy for bug reports. |

### Commands

| Command | Description |
|---------|-------------|
| `TyranoScript: Restart Language Server` | Restarts the language server (e.g. after swapping the server binary). |

---

## 🐛 Bug reports & feature requests

Please file bugs and feature requests on the [GitHub Issues](https://github.com/darallium/tyrano-parser/issues) page.
Attaching the communication log obtained with `tyranoscript.trace.server` set to `verbose` makes investigation much smoother.

---

## ❤️ Support the development

This extension and its language server are a free, personal-development project.
If it helped you, a little support via the link below goes a long way.

<a data-ofuse-widget-button href="https://ofuse.me/o?uid=101132" data-ofuse-id="101132" data-ofuse-size="large" data-ofuse-color="pink" data-ofuse-style="rectangle">OFUSEで応援を送る</a><script async src="https://ofuse.me/assets/platform/widget.js" charset="utf-8"></script>

---

## 📄 License

Released under the [MIT License](LICENSE).
