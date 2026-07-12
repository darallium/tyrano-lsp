# TyranoScript LSP

<p>
  <a href="https://marketplace.visualstudio.com/items?itemName=darallium.tyranoscript-language-server"><img src="https://img.shields.io/visual-studio-marketplace/i/darallium.tyranoscript-language-server?style=flat-square&label=Installs&color=0078d7" alt="Installs"></a>
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="License: MIT">
</p>

日本語 ・ [English](https://github.com/darallium/tyrano-lsp/blob/master/editors/code/README.cn.md) ・ [中文](https://github.com/darallium/tyrano-lsp/blob/claude/vscode-extension-readme-hmishl/editors/code/README.cnmd)

ノベルゲームエンジン **[ティラノスクリプト](https://tyrano.jp/)**（`.ks` ファイル）のシナリオ執筆を支援する VS Code 拡張機能です。
シンタックスハイライトに加え、Rust 製の言語サーバー **`tyrano-lsp`** による診断・補完・定義ジャンプ・参照検索を補助します。

![TyranoScript のシナリオファイルを VS Code で開いた様子](/editors/code/images/highlighting.png)

---

## 機能一覧

| 機能 | 内容 |
|------|------|
| [シンタックスハイライト](#-シンタックスハイライト) | タグ・ラベル・コメント・埋め込み JS/HTML の色分け |
| [リアルタイム診断](#-リアルタイム診断) | 未知タグ・未解決ジャンプ・素材欠落などを波線で通知 |
| [ホバー情報](#-ホバー情報) | タグ・パラメータ・ラベルの説明をポップアップ表示 |
| [入力補完](#-入力補完) | タグ名・パラメータ名・値の候補を提示 |
| [定義へ移動 / 参照検索](#-定義へ移動--参照検索) | ファイルをまたいだラベル・マクロの追跡 |
| [アウトライン / パンくず](#-アウトライン--パンくず) | ラベル・マクロ・キャラクターの一覧ナビゲーション |

---

### シンタックスハイライト

`.ks` ファイルを開くと、ティラノスクリプトの文法により各要素が色分けされます。
起動した瞬間から**即座に有効**になるため、シナリオを開いた瞬間から読みやすくなります。

![シンタックスハイライトの例](/editors/code/images/highlighting.png)

ハイライトされる主な要素:

- ラベル定義 — `*start|オープニング`
- タグ — `[bg storage=room.jpg time=1000]`
- 行頭コメント — `; ここはコメント`
- キャラクター名行 — `#あかね` / `#あかね:happy`
- `[iscript]` … `[endscript]` 内は **JavaScript としてハイライト可能**
- `[html]` … `[endhtml]` 内は **HTML として**ハイライト

---

### リアルタイム診断

入力・保存のたびにシナリオ全体を解析し、問題のある箇所に波線を表示します。
`.ks` ファイル単体ではなく プロジェクト全体（複数ファイル） を横断して検証するため、別ファイルのラベルや素材まで含めてチェックできます。

![診断と「問題」パネルの例](/editors/code/images/diagnostics.png)

検出できる問題の例:

| 診断コード | 内容 |
|-----------|------|
| `xsem-unknown-tag` | ビルトインにも定義済みマクロにも無い**未知のタグ**（`[teleprot]` など綴り間違い） |
| `sem-unknown-label-target` | 同一ファイル内に存在しない**ラベルへのジャンプ**（`target=*nowhere`） |
| `xsem-unknown-label-in-storage` | **別ファイルに存在しないラベル**への `[jump]`（`storage=scene2.ks target=*missing`） |
| `xsem-missing-asset` | `storage=` で指定した**画像・音声などの素材が見つからない** |
| `xsem-unknown-param` / `xsem-missing-param` | タグに対する**未知のパラメータ**、または**必須パラメータの欠落** |

問題は「問題（Problems）」パネル（`Ctrl+Shift+M`）に一覧表示され、クリックすると該当箇所へジャンプできます。

---

### ホバー情報

タグ・パラメータ・ラベルにマウスカーソルを重ねる（またはカーソルを置いて `Ctrl+K Ctrl+I`）と、その要素の説明がポップアップ表示されます。
とくに `[jump]` の飛び先ラベルにホバーすると、そのラベルがどのファイルで定義されているかまで解決して教えてくれます。

![ジャンプ先ラベルにホバーした様子](/editors/code/images/hover.png)

上の例では `target=*top` にホバーすることで、`*top` が `data/scenario/scene2.ks` に定義されたラベルであることが一目で分かります。

---

### 入力補完

タグ名・パラメータ名・パラメータ値の候補を自動で提示します。
`[` を入力した直後や、入力途中で `Ctrl+Space` を押すと候補リストが開き、**各候補の説明**も右側に表示されます。

![タグ名の補完候補](/editors/code/images/completion.png)

- **タグ名補完** — `[cha` まで入力すると `chara_show` / `chara_new` / `chara_hide` … を提示
- **パラメータ名補完** — そのタグが受け付けるパラメータのみを提示
- `[macro name=greet]` で定義したマクロも `[greet]` として補完されます

---

### 定義へ移動

ラベルやマクロの上で 定義へ移動（`F12`）・その場でのぞき見（`Alt+F12`）・参照を検索（`Shift+F12`）が使えます。
`storage=` で別ファイルを指定したジャンプも解決されるため、ファイルをまたいでシナリオの流れを追跡できます。

![別ファイルのラベル定義を Peek 表示した様子](/editors/code/images/definition.png)

上の例では、`first.ks` の `[jump storage=scene2.ks target=*top]` から `*top` の定義を辿り、`scene2.ks` の該当行をその場に展開しています。

---

### アウトライン

ファイル内の **ラベル・マクロ・キャラクター** が構造化され、サイドバーの「アウトライン」ビューと、
現在のファイル内位置についてはエディタ上部の「パンくず（breadcrumbs）」に一覧表示されます。
長いシナリオでも、目的のラベルへワンクリックで移動できます。

![アウトラインビューに表示されたラベル・マクロ・キャラクター](/editors/code/images/outline.png)

`Ctrl+Shift+O` でシンボル検索を開けば、ラベル名を入力して素早くジャンプすることも可能です。

---

### 設定項目

| 設定キー | 既定値 | 説明 |
|---------|--------|------|
| `tyranoscript.server.path` | `""` | `tyrano-lsp` 実行ファイルの絶対パス。空の場合は自動検出します。 |
| `tyranoscript.trace.server` | `off` | VS Code と言語サーバー間の通信ログレベル（`off` / `messages` / `verbose`）。不具合報告時に便利です。 |

---

## バグ報告・機能要望

不具合の報告や機能追加のご要望は、[GitHub の Issue](https://github.com/darallium/tyrano-lsp/issues) か、[Google Form](https://forms.gle/TeTvpCWH98CRFPUL9)までお寄せください。
`tyranoscript.trace.server` を `verbose` に設定して得られる通信ログを添えていただけると、調査がスムーズです。

---

## 開発の応援・寄付のお願い

この拡張機能と言語サーバーは個人開発による無償のプロジェクトです。
もし開発の助けになったと感じていただけたら、下記のリンクから応援していただけると励みになります。

<a data-ofuse-widget-button href="https://ofuse.me/o?uid=101132" data-ofuse-id="101132" data-ofuse-size="large" data-ofuse-color="pink" data-ofuse-style="rectangle">OFUSEで応援を送る</a><script async src="https://ofuse.me/assets/platform/widget.js" charset="utf-8"></script>

---

## ライセンス

[MIT License](LICENSE) の下で公開されています。
