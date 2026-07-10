;;* ------------------------------------
;* わたしとあなたの七日間問答
;* ------------------------------------


; マウス操作、キーボード操作、マウスのスワイプ操作無効化
[stop_keyconfig]

; 最初は右下のメニューボタンを非表示にする
[hidemenubutton]

; メッセージボックス非表示
[layopt layer="message" visible=false]

; キャラクター立ち絵設定
; pos_mode=falseで立ち絵の自動位置調整無効化。ptext="" で発言者の名前領域のptextを指定できる。
[chara_config ptext="chara_name_area" pos_mode="false"]

;* ------------------------------------
;* レイヤー表示設定
;* ------------------------------------

; 現在のレイヤー使用状況↓5~8以外は変化する可能性大。

;? 0: 動かす背景表示。
[layopt layer=0 visible=true]

;? 1: 顔出す背景。
[layopt layer=1 visible=true]

;? 2: 顔出す枠＆キャラ表示
[layopt layer=2 visible=true]

;? 3~6: 色々
[layopt layer=3 visible=true]
[layopt layer=4 visible=true]
[layopt layer=5 visible=true]
[layopt layer=6 visible=true]

;? 7: メッセージレイヤのキャラ表示
[layopt layer=7 visible=true]

;? 8: 気候・フィルター演出
[layopt layer=8 visible=true]

;? 9: 予備
[layopt layer=9 visible=true]

;? 10: システムボタン
[layopt layer=10 visible=true]

;* ------------------------------------
;* [plugin]
;* ------------------------------------

;? メッセージ縁取りプラグイン
[plugin name=message_edge]

;? UI一括変更プラグイン
[plugin name="theme_kopanda_23"]

;? ルールトランジションプラグイン
[plugin name=bg_rule]
[plugin name=mask_rule]

;? イントロつきbgmループプラグイン
[plugin name=intro_loop]

;? デバッグ用プラグイン
;! ビルド時にコメントアウトすること！
[plugin name=tsex]

;* ------------------------------------
;* [call]
;* ------------------------------------

;? ティラノスクリプトが標準で用意している便利なライブラリ群。
;? コンフィグ、CG、回想モードを使う場合は必須。
[call storage="tyrano.ks"]

;? スチル管理
[call storage="system/still_master.ks"]

;? BGM管理
[call storage="system/sound_info_master.ks"]

;? ファイルチェック用マクロ
[call storage="system/file_check.ks"]

;? モーダルウィンドウカスタム
[loadcss file="./data/others/css/mystyle.css"]

;? layermodeタグで背景のみに適用したい場合に指定するレイヤー
[loadcss file="./data/others/css/layer_blend_only_background.css"]

;? その他マクロ
[call storage="system/macro.ks"]

;? 背景・マスクマクロ
[call storage="system/bg_mask_macro.ks"]

;? アニメーション先読み込み
[call storage="system/anim.ks"]

;? 素材画像の先読み込み
[call storage="system/preload.ks"]

;? シナリオジャンプ用マクロ
[call storage="system/jump_scenario.ks"]

;? 整列ルビマクロ
[call storage="system/sorted_ruby.ks"]

;? 傍点マクロ
[call storage="system/emphasis_dots.ks"]

;! デバッグ時のみ実行
[eval exp="sf.is_initialized = false" cond="TYRANO.kag.config['debugMenu.visible'] == 'true'"]

;* ------------------------------------
;* 変数の初期化
;* ------------------------------------

[iscript]
    if(!sf.is_initialized){
        sf.is_clear_normal = false;
        sf.is_clear_true = false;
        f.choice = true;

        sf.is_initialized = true;
    }
[endscript]

; 念のため（特に意味なく）置く、レイヤー要素消去タグ
[cm]
[clearfix]

; フォントサイズ変更
[deffont size=26]

; フォントの変更
[font face="stained-glass-font"]

[message_config line_spacing="0" speech_bracket_float="true"]

;! デバッグ時のみ実行
[if exp="TYRANO.kag.config['debugMenu.visible'] == 'true'"]
    ; 各パラメータを設定
    [eval exp="sf.is_clear_normal = true"]
    [eval exp="sf.is_clear_true = false"]
    [eval exp="f.choice = true"]
[endif]

; タイトル画面へ移動
[jump storage="title.ks"]
[s]
