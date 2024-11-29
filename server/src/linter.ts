import { Node, TagNode, TokenType } from "./parser";

//Lintエラーの情報
interface LintError {
  message: string;

  line: number;

  column: number;
}

export class TyranoScriptLinter {
  // tagの定義
  private tagDefinitions: Record<string, { vital: string[] }> = {
    // tagの定義を追加
    "3d_init": { vital: [] },
    "3d_model_new": { vital: ["name", "storage"] },
    "3d_show": { vital: ["name"] },
    "3d_hide": { vital: ["name"] },
    "3d_delete": { vital: ["name"] },
    "3d_event": { vital: ["name"] },
    "3d_camera": { vital: [] },
    "3d_scene": { vital: [] },
    "3d_anim": { vital: ["name"] },
    "3d_gyro": { vital: [] },
    "3d_new_group": { vital: ["name"] },
    "3d_add_group": { vital: ["name", "group"] },
    anim: { vital: [] },
    xanim: { vital: ["keyframe"] },
    stop_xanim: { vital: [] },
    bg: { vital: ["storage"] },
    bg2: { vital: ["storage"] },
    bgmovie: { vital: ["storage"] },
    wait_bgmovie: { vital: [] },
    stop_bgmovie: { vital: [] },

    bgcamera: { vital: [] },
    stop_bgcamera: { vital: [] },

    body: { vital: [] },

    breakgame: { vital: [] },

    button: { vital: [] },
    call: { vital: [] },
    chara_mod: { vital: ["name"] },
    chara_new: { vital: ["name", "storage"] },
    chara_show: { vital: ["name"] },
    chara_hide: { vital: ["name"] },
    chara_move: { vital: ["name"] },
    chara_ptext: { vital: [] },
    chara_config: { vital: [] },

    chara_face: { vital: ["name", "face", "storage"] },

    chara_part: { vital: ["name"] },
    chara_layer: { vital: ["name", "part", "id"] },

    chara_hide_all: { vital: [] },

    clearfix: { vital: [] },
    clearstack: { vital: [] },
    clearsysvar: { vital: [] },
    clearvar: { vital: [] },

    close: { vital: [] },

    clickable: { vital: ["width", "height"] },
    cm: { vital: [] },
    ct: { vital: [] },
    current: { vital: ["layer"] },
    cursor: { vital: [] },
    deffont: { vital: [] },
    delay: { vital: [] },
    resetdelay: { vital: [] },
    dialog: { vital: [] },
    dialog_config: { vital: [] },
    dialog_config_ok: { vital: [] },
    dialog_config_ng: { vital: [] },

    edit: { vital: ["name"] },

    emb: { vital: ["exp"] },
    endhtml: { vital: [] },
    endlink: { vital: [] },
    endmacro: { vital: [] },
    endscript: { vital: [] },
    endif: { vital: [] },
    endignore: { vital: [] },
    endnowait: { vital: [] },

    erasemacro: { vital: ["name"] },
    eval: { vital: ["exp"] },
    fadeinbgm: { vital: ["storage"] },
    fadeinse: { vital: ["storage", "time"] },
    fadeoutbgm: { vital: [] },
    fadeoutse: { vital: [] },

    filter: { vital: [] },
    free: { vital: ["layer", "name"] },
    free_filter: { vital: [] },
    freeimage: { vital: ["layer"] },
    freelayer: { vital: ["layer"] },

    font: { vital: [] },
    fuki_start: { vital: [] },
    fuki_stop: { vital: [] },

    fuki_chara: { vital: ["name"] },

    glyph: { vital: [] },
    glyph_auto: { vital: [] },
    glyph_skip: { vital: [] },
    glink: { vital: [] },
    glink_config: { vital: [] },
    graph: { vital: ["storage"] },
    html: { vital: [] },
    if: { vital: ["exp"] },
    elsif: { vital: ["exp"] },

    ignore: { vital: ["exp"] },

    image: { vital: ["layer", "x", "y"] },

    iscript: { vital: [] },

    jump: { vital: [] },

    kanim: { vital: ["keyframe"] },

    keyframe: { vital: ["name"] },
    endkeyframe: { vital: [] },
    frame: { vital: ["p"] },

    label: { vital: [] },
    lang_set: { vital: ["name"] },

    layopt: { vital: ["layer"] },

    layermode: { vital: [] },
    layermode_movie: { vital: ["video"] },

    l: { vital: [] },
    loadcss: { vital: ["file"] },
    loadjs: { vital: ["storage"] },
    loading_log: { vital: [] },

    locate: { vital: [] },

    macro: { vital: ["name"] },
    mark: { vital: [] },

    endmark: { vital: [] },

    message_config: { vital: [] },

    mode_effect: { vital: [] },

    movie: { vital: ["storage"] },

    nolog: { vital: [] },
    endnolog: { vital: [] },

    nowait: { vital: [] },

    p: { vital: [] },

    ptext: { vital: ["layer", "x", "y"] },

    plugin: { vital: ["name"] },

    position: { vital: ["layer"] },
    position_filter: { vital: [] },

    preload: { vital: ["storage"] },
    pushlog: { vital: ["text"] },

    quake: { vital: ["time"] },
    quake2: { vital: [] },
    qr_config: { vital: [] },
    qr_define: { vital: ["url"] },

    r: { vital: [] },

    reset_camera: { vital: [] },
    resetfont: { vital: [] },

    return: { vital: [] },

    s: { vital: [] },

    save_img: { vital: [] },

    savesnap: { vital: ["title"] },
    scene: { vital: [] },
    screen_full: { vital: [] },
    set_resizecall: { vital: ["storage"] },
    seopt: { vital: [] },
    showload: { vital: [] },
    showlog: { vital: [] },
    showmenu: { vital: [] },
    showmenubutton: { vital: [] },
    showsave: { vital: [] },

    skipstart: { vital: [] },
    skipstop: { vital: [] },
    sleepgame: { vital: [] },
    awakegame: { vital: [] },
    speak_on: { vital: [] },
    speak_off: { vital: [] },

    stopanim: { vital: ["name"] },

    stopbgm: { vital: [] },
    stopse: { vital: [] },

    stop_keyconfig: { vital: [] },
    start_keyconfig: { vital: [] },

    system: { vital: [] },
    tag: { vital: [] },
    text: { vital: [] },
    title: { vital: ["name"] },

    trace: { vital: ["exp"] },
    trans: { vital: ["time", "layer"] },

    vchat_in: { vital: [] },
    vchat_config: { vital: [] },
    vchat_chara: { vital: ["name"] },

    voconfig: { vital: ["sebuf", "vostorage"] },
    vostart: { vital: [] },
    vostop: { vital: [] },

    wa: { vital: [] },

    wait: { vital: ["time"] },
    wait_cancel: { vital: [] },

    wait_camera: { vital: [] },

    web: { vital: ["url"] },
    wbgm: { vital: [] },
    wse: { vital: [] },
  };

  lint(nodes: Node[]): LintError[] {
    const errors: LintError[] = [];
    this.checkTagDefinitions(nodes, errors);
    // その他のLintルールを追加
    this.checkNest(nodes, errors);

    return errors;
  }

  private checkTagDefinitions(nodes: Node[], errors: LintError[]) {
    for (const node of nodes) {
      if (node.type === TokenType.Tag) {
        const tagNode = node as TagNode;
        const tagDefinition = this.tagDefinitions[tagNode.name];

        if (tagDefinition) {
          for (const vitalParam of tagDefinition.vital) {
            const param = tagNode.parameters.find((p) => p.name === vitalParam);

            if (!param || !param.value) {
              //必須パラメータがない場合にエラーを追加

              errors.push({
                message: `[${tagNode.name}] タグに必須パラメータ ${vitalParam} がありません`,
                line: tagNode.line,
                column: tagNode.column,
              });
            }
          }
        } else {
          //未定義のタグの場合にエラーを追加

          errors.push({
            message: `未定義のタグ [${tagNode.name}] が使用されています`,

            line: tagNode.line,

            column: tagNode.column,
          });
        }
      }

      //子ノードも再帰的にチェック

      if ("children" in node) {
        this.checkTagDefinitions(node.children, errors);
      }
    }
  }

  private checkNest(nodes: Node[], errors: LintError[]) {
    //ネストのチェックを行う
    const stack: { tag: string; line: number; column: number }[] = [];

    for (const node of nodes) {
      if (node.type === TokenType.Tag) {
        const tagNode = node as TagNode;

        if (tagNode.name.startsWith("end")) {
          // スタックから対応する開始タグをポップ

          const startTag = stack.pop();

          if (!startTag || startTag.tag !== tagNode.name.slice(3)) {
            // 対応する開始タグがない場合はエラー

            errors.push({
              message: `閉じタグ [${tagNode.name}] に対応する開始タグがありません`,

              line: tagNode.line,

              column: tagNode.column,
            });
          }
        } else if (
          tagNode.name === "if" ||
          tagNode.name === "macro" ||
          tagNode.name === "iscript" ||
          tagNode.name === "html"
        ) {
          //開始タグならスタックに積む

          stack.push({
            tag: tagNode.name,
            line: tagNode.line,
            column: tagNode.column,
          });
        }
      }
      // 子ノードがあれば再帰的にチェック
      if ("children" in node) {
        this.checkNest(node.children, errors);
      }
    }

    // スタックに残っているタグがあればエラー

    while (stack.length > 0) {
      const startTag = stack.pop();
      if (startTag) {
        errors.push({
          message: `開始タグ [${startTag.tag}] に対応する閉じタグがありません`,

          line: startTag.line,

          column: startTag.column,
        });
      }
    }
  }
}
