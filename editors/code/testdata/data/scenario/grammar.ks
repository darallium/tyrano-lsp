; grammar fixture: quote handling and html/iscript blocks
*start

; --- quotes: three kinds, brackets hidden inside quotes ---
[ptext text="a b" name='c d' exp=`e f`]
[ptext text="[[あ]]" y=`x]y`]
[eval exp="f.a=1" time=200]
[ptext text="unterminated
plain line after unterminated quote[l]

; --- html block with parameters (bug report case) ---
[html top="0" left="0"]
    <div style="width: 1280px; height: 720px;">
        <img class="button_home" src="data/image/gallery_mode_common/button_home.png">
    </div>
[endhtml]
text after endhtml[l]

[html2]
not a block line

; --- iscript block with parameters ---
[iscript stop=true]
var a = 1;
[endscript foo=1]
text after endscript[l]

@iscript
var b = 2;
@endscript
the end
