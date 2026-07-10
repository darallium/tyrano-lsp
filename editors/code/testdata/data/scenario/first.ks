*start|オープニング
; このシーンはテスト用のサンプルです
[macro name=greet]
こんにちは[p]
[endmacro]

[greet]

[iscript]
f.playCount = (f.playCount || 0) + 1;
[endscript]

[jump storage=scene2.ks target=*top]
