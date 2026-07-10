## tyrano-parser-generator要件定義（案）

### 1. 目的

* **LALR(1) 文法**（bison 互換に近い記法）から **Rust のパーサ**を生成する。
* TyranoScript の曖昧性・エラー耐性の要求に対応できる “拡張点” を持つが、**文法は必ず grammar ファイルで与える**（ハードコード依存を避ける）。
* 生成物は後段（tyrano-analyzer / tyrano-formatter）で **CFG（生成規則）を追跡**できる形で、CST/AST を構築可能にする。

---

## 2. スコープ（過剰な一般化を避けるための線引き）

### やる（MUST）

* LALR(1) パーサ生成（LR(1) → LALR(1) マージ）。
* bison 互換に近い grammar 形式の読み取り（宣言部・規則部・付随コード）。
* 競合（shift/reduce, reduce/reduce）の検出と、**優先順位・結合規則**による解決（bison の `%left/%right/%nonassoc` 相当）。
* **エラー復帰**の仕組み（bison の `error` トークン＋拡張 directive）。
* **CST を確実に生成**（CFG追跡のための production id / symbol id / span / 元位置）。
* “Macro 引数の型分類（数値/boolean/文字列）を後段で推論できる” 情報を **CST/メタデータとして保持**し、必要なら明示注釈も許可する。

### やらない（WON’T：少なくとも初期）

* GLR/Earley など別アルゴリズム（LALR(1)に集中）。
* 何でもできる汎用IRコンパイラ（あくまで「パーサ生成」に集中）。
* “ユーザーが任意Rustコードを規則内に埋め込み放題” の方向（安全性と設計を壊すので、埋め込みは制限するか段階導入）。

---

## 3. 入力・出力

### 入力

* `*.tyrano_parser_generator`（仮）: bison 互換風 grammar ファイル

  * 例：`%token`, `%start`, `%type`, `%%` 区切り、規則、（限定的な）アクション指定
* （任意）Rust 側のサポートコード：AST ノード定義、トークン型、ユーティリティ等

### 出力（生成される Rust）

* `parser.rs`：テーブル駆動 LALR(1) パーサ本体
* `tables.rs`：action/goto テーブル（圧縮済み配列など）
* `cst.rs`：CST ノード型（ProductionId, SymbolId, Span, 子ノード）
* `grammar_meta.rs / grammar.json`：CFG追跡用メタデータ（規則一覧、ファイル位置、優先順位）
* （任意）`token.rs`：TokenKind enum / Span / Lexer trait（プロジェクト方針により切替）

### 追加のデバッグ出力（開発・CI向け）

* `automaton.dot`：状態遷移の可視化
* `conflicts.txt`：競合の詳細（どの状態で、どの lookahead で、どの規則が衝突したか）
* `first_follow.json`：nullable/FIRST/FOLLOW の計算結果

---

## 4. grammar ファイル仕様（bison互換＋必要最小限の拡張）

### bison互換で最低限サポート

* `%start <nonterminal>`
* `%token`（名前、必要なら型）
* `%type`（非終端の型）
* `%left/%right/%nonassoc`（優先順位）
* ルール定義：`A : B C | D ;`
* 擬似トークン：`error`

### 拡張（tyrano_parser_generatorとして追加して良いもの）

#### (A) “CFG追跡”のためのメタ情報

* 各規則に安定IDを付与できる仕組み（明示 or 自動）

  * 例：`stmt[rule_id="stmt.command"] : command NL ;`
* 規則・シンボルに “役割タグ” を付ける（後段の analyzer が使う）

  * 例：`macro_arg[tag="macro_arg"] : ... ;`

#### (B) エラー復帰 directive（TyranoScript想定の「文境界」を表現）

* bison の `error` だけだと復帰戦略が弱いので、**同期点（sync set）**を宣言できるようにする。

  * 例：

    * `%recover stmt: sync(NL, ';', RBRACK) ;`
    * あるいは規則末尾に：`| error NL %recover(sync=NL);`
* “捨てる/挿入する/置換する” の最小限ポリシー指定

  * `discard_until(...)`
  * `insert_if_missing(...)`（例：`)` や `]` の補完）

#### (C) Macro 引数の “明示” と “推論” を両立するための下支え

* トークン/非終端に **型タグ**（数値/boolean/文字列/不明）を付けられる

  * 例：`%token <Num> INT`, `%token <Bool> TRUE FALSE`, `%token <Str> STRING`
* 明示注釈構文を grammar で許可（言語仕様側）

  * 例：`arg : IDENT ':' type '=' expr` のように **文法で**表現
* 推論は後段でやるが、CST 側に以下を残す：

  * リテラル種別（Num/Bool/Str）
  * production id（どの規則で “macro_arg” になったか）
  * span と token 列（エラーメッセージ・整形に必須）

> ポイント：型推論エンジン自体を parser generator に入れず、**推論可能な材料（CST＋CFGメタ）を確実に出す**のが “過剰な一般化を避ける” うえで安定です。

---

## 5. 生成するパーサの実行モデル（CST/AST方針）

### MUST：CST生成（デフォルト）

* 規則還元（reduce）時に **CST Node** を積む。
* Node は最低限これを持つ：

  * `production_id`
  * `lhs_symbol_id`
  * `children`（Token leaf or Node）
  * `span`（開始〜終了）
* Token leaf も `token_kind + lexeme(optional) + span` を持つ。

### OPTIONAL：AST生成（段階導入）

* 初期は **CST固定**が安全（設計が固まるまで）。
* 次段階で AST 生成を導入するなら、bison風の自由記述アクションよりも：

  * `@action(name)` のような **名前参照**（生成コードがユーザー関数を呼ぶ）
  * もしくは “イベントストリーム”（reduceイベントを吐き、別段でAST構築）
    …のどちらかが、Rust的に破綻しにくいです。

---

## 6. エラー処理要件（TyranoScript寄りだが文法駆動）

### 目標

* 1つのエラーで解析が止まらず、**可能な限り後続を解析して複数エラー報告**。
* 期待トークン（expected set）を出す。
* “どの規則でどこまで読めていたか” が CFG追跡で分かる。

### 実装に必要なもの

* state ごとの expected token 推定（action table から算出）
* 回復戦略（最低限）

  1. **panic-mode**：同期点まで token を捨てる
  2. **single-token insertion**：閉じ括弧や区切りなど、明示的に許可されたものだけ挿入
  3. `error` 非終端を使った phrase-level recovery（bison互換）

### “TyranoScript固有”への寄せ方（ハードコードしない）

* `profile` という概念だけ用意し、デフォルト同期点や推奨ルールを **宣言で注入**できるようにする：

  * 例：`%profile "line_oriented"` → 同期点候補に `NL` を含める
  * ただし実際の同期点は `%recover` で grammar 側が最終決定

---

## 7. マルチステージ・パイプライン設計（lrama 参照の方針）

「一枚岩の “読んで即生成”」にせず、各段で IR を持って検証・可視化できる形にします。

### ステージ案（tyrano_parser_generator内部）

1. **Grammar Parse（CST）**

   * grammar ファイル自体を解析（位置情報つき）
2. **Lowering（Grammar IR）**

   * Symbol/Production の正規化、優先順位、型タグ、directive を集約
3. **Validation**

   * 未定義シンボル、到達不能規則、左再帰の注意喚起、優先順位未設定による衝突予兆など
4. **Analysis**

   * nullable / FIRST / FOLLOW
5. **LR(1) Automaton**

   * closure/goto、canonical LR(1) item sets
6. **LALR(1) Merge**

   * core でマージ、lookahead を合成
7. **Conflict Resolution**

   * precedence/assoc、明示解決directive（必要なら）
8. **Table Build & Compression**

   * action/goto テーブル、エンコード（配列/bitset）
9. **Codegen**

   * Rust パーサ + CST + メタデータ出力

> 各ステージは `--emit stage_name=json` などで中間生成物を出せると、曖昧性・衝突の解析が圧倒的に楽になります。

---

## 8. Rust実装概要（crate構成と主要データ構造）

### crate分割（推奨）

* `tyrano_parser_generator`（bin）：CLI、ファイルI/O、オプション
* `tyrano_parser_generator_grammar`：grammar ファイルのパーサ（位置情報付きCST）
* `tyrano_parser_generator_core`：Grammar IR、FIRST/FOLLOW、オートマトン生成、競合解析
* `tyrano_parser_generator_codegen_rust`：Rustコード生成（テンプレート + 生成IR）
* `tyrano_parser_generator_runtime`：生成パーサが依存する最小ランタイム（Span、Token trait、エラー型など）

### コア構造（例）

* `SymbolId(u32)`, `ProductionId(u32)`, `StateId(u32)`
* `Symbol { kind: Terminal|Nonterminal, name, type_tag?, origin_span }`
* `Production { lhs, rhs: Vec<SymbolId>, prec?, rule_id?, origin_span }`
* `Item { production, dot, lookahead: TerminalSet }`
* `State { kernel: Vec<ItemCore>, closure: Vec<Item>, transitions }`
* `ParseTables { action[state][terminal], goto[state][nonterminal] }`

---

## 9. CLI（最低限の操作性）

* `tyrano_parser_generator build grammar.tyrano_parser_generator -o src/generated/`
* `tyrano_parser_generator check grammar.tyrano_parser_generator`（衝突/未定義/到達不能を検査、失敗でexit≠0）
* `tyrano_parser_generator debug grammar.tyrano_parser_generator --emit automaton.dot --emit conflicts.txt`
* `tyrano_parser_generator fmt-grammar grammar.tyrano_parser_generator`（将来：文法ファイル整形）

CIで使うなら：

* `--fail-on-conflict`
* `--expect-shift-reduce N`（bisonの `%expect` 相当。固定数以上ならエラー）

---

## 10. 最初のマイルストーン（実装順の提案：設計固定に効く順）

1. Grammar CST → Grammar IR（位置情報とID付与まで）
2. FIRST/FOLLOW + LR(1) canonical + LALR merge
3. 競合レポート（これがないと曖昧性と戦えない）
4. Rustコード生成（まずはテーブル駆動 + CST構築固定）
5. エラー復帰（`error` + `%recover`）
6. CFG追跡メタ出力の充実（production/tag/role）

---

# まとめ（この要件の“芯”）

* **TyranoScriptに寄せすぎず**、しかし必要な“行指向・タグ指向の復帰”を **grammar directive** で表現できるようにする。
* **推論**は parser generator に抱え込まず、**推論可能なCST＋CFGメタ（安定ID/位置/規則タグ）**を確実に吐く。
* **マルチステージIR**で「衝突・曖昧性・復帰」を観測可能にする（これが最終的な品質を決めます）。
