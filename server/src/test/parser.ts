import { TyranoScriptParser } from "../parser";

const parser = new TyranoScriptParser();
const script = `
;ﾕﾆﾊﾞｰｻﾙﾄﾗﾝｼﾞｼｮﾝﾌﾟﾗｸﾞｲﾝについて
;[bg_rule]
;背景=背景画像を指定する\nデフォルトはblack, 画像ファイル名
;ルール=ルール画像, 画像ファイル名
;時間=トランジション時間\nデフォルトは1000
;storage  :【必須】背景画像ファイル。data/bgimageフォルダに配置。colorを指定する場合のみ省略してよい。
;rule     :【必須】ルール画像ファイル。data/image/bg_ruleフォルダに配置。
;time     :【任意】切り替え時間(ミリ秒)。
;clickskip:【任意】切り替え中にクリックすることで演出をスキップできるようにするか。trueかfalseで指定する。デフォルトはfalse。
;wait     :【任意】切り替え完了まで次のタグへの進行を待機するか。trueかfalseで指定する。デフォルトはtrue。
;reverse  :【任意】ルール画像の白黒を逆向きに処理するかどうか。trueかfalseで指定する。デフォルトはfalse。
;folder      :【任意】背景画像が入っているフォルダを指定する。デフォルトはbgimage。
;rule_folder :【任意】ルール画像が入っているフォルダを指定する。デフォルトはimage/bg_rule_image。

;* --------------------------------------------------
;* [background]
;* --------------------------------------------------

;* 背景変更マクロ

;? bg   : 背景種類
;? rule : ルール画像番号 未指定で左から右へのブラインド
;? time : 切り替え時間 未指定で1000ms

; 以下、過去作(2024年10月時点)での作業経験がある方向け
; アルゴリズムが大幅に変更されています！
; timeパラメータの指定が変わっています。元のtranstimeパラメータがtimeに変更されました。
; ファイル指定も、画像ファイル名に"_dt"を付ける必要がなくなりました。むしろ付けるとエラーとなるので注意してください。
[macro name="background"]

    ; ルールが省略された場合はblindに設定
    [iscript]
        if("rule" in mp) {
            TYRANO.kag.variable.tf.trans_bg_rule = mp.rule;
        }
        else {
            TYRANO.kag.variable.tf.trans_bg_rule = "blind";
        }
    [endscript]

    ; 拡張子を付ける
    [eval exp="tf.background_storage = mp.bg + '.png'"]

    ; ファイルの存在確認
    [fileCheck file="&tf.background_storage" location="bgimage"]

    ; ファイルが存在すれば背景変更
    [if exp="TYRANO.kag.variable.sf.exsist_file"]
        [bg_rule storage="&tf.background_storage" rule="&TYRANO.kag.variable.tf.trans_bg_rule + '.png'" time=%time|1000 wait=%wait|true]
    [endif]
[endmacro]


;ﾕﾆﾊﾞｰｻﾙﾄﾗﾝｼﾞｼｮﾝﾏｽｸﾌﾟﾗｸﾞｲﾝについて
;[mask_rule]
;graphic  :【必須】マスク画像ファイル。data/imageフォルダが基準。colorを指定する場合のみ省略できる。
;color    :【任意】マスク色。graphicに代えて色を指定することができる。
;rule     :【必須】ルール画像ファイル。image/bg_rule_imageフォルダに配置。
;time     :【任意】切り替え時間(ミリ秒)。デフォルトは1000。
;reverse  :【任意】ルール画像の白黒を逆向きに処理するかどうか。trueかfalseで指定する。デフォルトはfalse。
;folder      :【任意】マスク画像が入っているフォルダを指定する。デフォルトはimage。
;rule_folder :【任意】ルール画像が入っているフォルダを指定する。デフォルトはimage/bg_rule_image。


;マスクマクロ
;rule=ﾕﾆﾊﾞｰｻﾙﾄﾗﾝｼﾞｼｮﾝ方法番号 未指定か値にfadeでフェード
;time=切り替え時間
;glaphic=マスク画像 未指定か値にblackで黒
[macro name="masking"]
[iscript]
    TYRANO.kag.variable.tf.univ_transition_mask = false;
    if("rule" in mp) {
        if (mp.rule != 'fade') {
            TYRANO.kag.variable.tf.trans_mask_rule = mp.rule;
            TYRANO.kag.variable.tf.univ_transition_mask = true;
        }
    }
    if("graphic" in mp) {
        if (mp.graphic != 'black') {
            TYRANO.kag.variable.tf.trans_graphic_img = mp.graphic;
        }
    }
    else{
        TYRANO.kag.variable.tf.trans_graphic_img = "black";
    }
[endscript]
[if exp="TYRANO.kag.variable.tf.univ_transition_mask"]
    [mask_rule rule="&TYRANO.kag.variable.tf.trans_mask_rule + '.png'" time=%time|1000 graphic="&TYRANO.kag.variable.tf.trans_graphic_img + '.png'"]
[else]
    [mask time=%time|1000 graphic="&TYRANO.kag.variable.tf.trans_graphic_img + '.png'"]
[endif]
[endmacro]


;マスク消去マクロ
;rule=ﾕﾆﾊﾞｰｻﾙﾄﾗﾝｼﾞｼｮﾝ方法番号 未指定か値にfadeでフェード
;time=切り替え時間
[macro name="dis_masking"]
[iscript]
    TYRANO.kag.variable.tf.univ_transition_maskoff = false;
    if("rule" in mp) {
        if (mp.rule != 'fade'){
            TYRANO.kag.variable.tf.trans_maskoff_rule = mp.rule;
            TYRANO.kag.variable.tf.univ_transition_maskoff = true;
        }
    }
[endscript]
[if exp="TYRANO.kag.variable.tf.univ_transition_maskoff"]
    [mask_off_rule rule="&TYRANO.kag.variable.tf.trans_maskoff_rule + '.png'" time=%time|1000]
[else]
    [mask_off time=%time|1000]
[endif]
[endmacro]

[iscript]
// JavaScriptのコード
[endscript]
; single-line comment
/* multi-line comment
   second line */
[tag storage="value"]
テキスト
[tag2]
/* multi-line
comment */
[tag3]
; comment
`;

const tokens = parser.parse(script);

console.log(tokens);
