# Tyranoscript LSP

> [!IMPORTANT]
> news: リポジトリは完全にリワークされました。
> syntax 関連を大幅にモダン化したため,非常に高速かつ強力なエディタ支援が提供できるようになりました．

Tyranoscript LSPは、ビジュアルノベルで使用されるスクリプト言語であるTyranoscriptのためのLanguage Server Protocol Server及びVSCode拡張です。
このプロジェクトは、コード補完、エラーチェックなどの機能を提供し、開発体験を向上させることを目的としています。


# 特徴

このリポジトリでは， tyranoscript の **完全な言語解析** を行います．
言語レベルで本来持ち合わせている参照などを含め，非常に強力な支援を行います．
オリジナルのリポジトリに含まれるバグまで再現しているため，互換性はばっちりです．

## 機能
- [x] **コード補完**: タイピング中にコードのサジェストでます。
- [x] **エラーチェック**: スクリプト内のエラーを特定し、修正します。
- [x] **シンタックスハイライト**: シンタックスに特化した色で読みやすさを向上させます。
- [x] **定義へ移動**: 関数や変数の定義に素早く移動できます。
- [x] **ホバー情報**: 関数や変数に関する情報をホバーで取得します。

## インストール
1. リポジトリをクローンします: `git clone https://github.com/darallium/tyrano-lsp.git`
2. プロジェクトディレクトリに移動します: `cd tyranoscript-lsp`
3. 言語処理系をビルドします: `cargo build --all`
3. 依存関係をインストールします: `npm install`
4. サーバーを起動します: `npm start`

## コントリビューション
鉞は自由に飛ばしてください．

## ライセンス
このプロジェクトはMITライセンスの下でライセンスされています。詳細は[LICENSE](LICENSE)ファイルをご覧ください。

This project uses the tyrano.tmLanguage.json file from https://github.com/orukRed/tyranosyntax, licensed under the MIT License.


