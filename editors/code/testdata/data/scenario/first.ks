*start|オープニング
; このシーンはテスト用のサンプルです
[macro name=greet]
こんにちは[p]
[endmacro]

[greet]

[iscript]
f.playCount = (f.playCount || 0) + 1;
[endscript]

[loadjs storage=navigation.js]
[custom_tag]

[jump storage=scene2.ks target=*top]
