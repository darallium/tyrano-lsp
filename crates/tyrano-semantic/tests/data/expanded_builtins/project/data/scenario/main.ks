*start
; correct usages of newly added builtin tags: no diagnostics expected
[fadeinbgm storage=theme.ogg time=1000]
[chara_move name=akane time=300 left="+=200"]
[dialog type=confirm text="continue?"]
[mask effect=zoomIn color=0x336699]
[skipstart]
[position_filter layer=message0 blur=4]
[3d_init]
[3d_box_new name=crate pos="0,10,0" color=0x00ff00]
; incorrect usages: each line below must produce one diagnostic
[dialog type=bogus]
[mask effect=flyIn]
[quake]
[pausebgm buf=abc]
[pausebgm typo_param=1]
[s]
