*TITLE


[cm]
[clearstack]

[bg storage="Creo.png" time="500"]
[wait time="2000"]

*skip_creo

[bg storage="white.png" time="1000"]

*title

; 画面を再構成
[bg storage="title.png" time="1000"]
[cg storage="title.png"]

; ロゴを表示
[image layer="0" name="title_logo" visible="true" x="0" y="0" width="1280" height="720" storage="../image/title/title_logo.png" time="1000"]

[stop_keyconfig]


*title_cg
; 記録以外のボタンを表示

; 開始ボタン
[button x="50" y="370" width="250" name="title_button" graphic="title/button/title_start.png" enterimg="title/button/title_start_hover.png" target="gamestart"]

; ロードボタン
[button x="110" y="430" width="250" name="title_button" graphic="title/button/title_load.png" enterimg="title/button/title_load_hover.png" role="load"]

; 設定ボタン
[button x="170" y="490" width="250" name="title_button" graphic="title/button/title_config.png" enterimg="title/button/title_config_hover.png" role="sleepgame" storage="config.ks"]

; 閉じるボタン
[button x="220" y="550" width="250" name="title_button" graphic="title/button/title_close.png" enterimg="title/button/title_close_hover.png" storage="EXIT.ks"]

;ギャラリー
[if exp="sf.is_clear_true || sf.is_clear_normal"]
    [button x="270" y="610" width="250" name="title_button" graphic="title/button/title_gallery.png" enterimg="title/button/title_gallery_hover.png" role="sleepgame" storage="cg.ks"]
[else]
    [image folder="image" storage="title/button/archive_3.png" layer="0" name="title_button" width="250" x="515" y="560" visible="true" time="0"]
[endif]

[s]


;* ------------------------------------
;* はじめからを押したときの処理
;* ------------------------------------
*gamestart

; マウス操作、キーボード操作、マウスのスワイプ操作無効化
; タイトル画面のボタン類を消す
[free layer="0" name="title_button" time="0"]
[cm]
[clearfix]

; imageを消す
[freeimage layer="0" time="1000" wait="false"]

; 画面明転
[background bg="white" time="0"]

; BGMが掛かっていたら消す
[fadeoutbgm time="1000"]

[wait time="1000"]

; 一番最初のシナリオファイルへジャンプする
[jump_scenario storage="7DaysQA/prologue.ks"]
