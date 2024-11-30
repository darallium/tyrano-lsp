# Tyranoscript LSP
Tyranoscript LSPは、ビジュアルノベルで使用されるスクリプト言語であるTyranoscriptのためのLanguage Server Protocol Server及びVSCode拡張です。
コード構造を解析するために抽象構文木（AST）を構築しているので、ifの対応なども検出します。
このプロジェクトは、コード補完、エラーチェックなどの機能を提供し、開発体験を向上させることを目的としています。嘘です。俺がneovimを使いたいだけです。

## 機能
- [ ] **コード補完**: タイピング中にコードのサジェストでます。
- [ ] **エラーチェック**: スクリプト内のエラーを特定し、修正します。
- [x] **シンタックスハイライト**: シンタックスに特化した色で読みやすさを向上させます。
- [ ] **定義へ移動**: 関数や変数の定義に素早く移動できます。
- [ ] **ホバー情報**: 関数や変数に関する情報をホバーで取得します。

## インストール
1. リポジトリをクローンします: `git clone https://github.com/darallium/tyrano-lsp.git`
2. プロジェクトディレクトリに移動します: `cd tyranoscript-lsp`
3. 依存関係をインストールします: `npm install`
4. サーバーを起動します: `npm start`

## コントリビューション
鉞は自由に飛ばしてください．

## ライセンス
このプロジェクトはMITライセンスの下でライセンスされています。詳細は[LICENSE](LICENSE)ファイルをご覧ください。

This project uses the tyrano.tmLanguage.json file from https://github.com/orukRed/tyranosyntax, licensed under the MIT License.


