// トークンの種類
enum TokenType {
  Tag,
  Text,
  Comment,
  Macro,
  EndMacro,
  If,
  ElseIf,
  Else,
  EndIf,
  Iscript,
  Endscript,
  Html,
  Endhtml,
  InlineLanguage,
}

// トークン
interface Token {
  type: TokenType;
  value: string;
  line: number;
  column: number;
  children: Node[];
}

// タグのパラメータ
interface TagParameter {
  name: string;
  value: string;
  children: Node[];
}

// タグのASTノード
interface TagNode {
  type: TokenType.Tag;
  name: string;
  parameters: TagParameter[];
  inheritParams: boolean;
  line: number;
  column: number;
  children: Node[];
}

interface TextNode {
  type: TokenType.Text;
  value: string;
  line: number;
  column: number;
  children: Node[];
}

interface CommentNode {
  type: TokenType.Comment;
  value: string;
  line: number;
  column: number;
  children: Node[];
}

interface MacroNode {
  type: TokenType.Macro;
  name: string;
  line: number;
  column: number;
  children: Node[];
}
interface EndMacroNode {
  type: TokenType.EndMacro;
  line: number;
  column: number;
  children: Node[];
}

interface IfNode {
  type: TokenType.If;
  exp: string;
  line: number;
  column: number;
  children: Node[];
}

interface ElseIfNode {
  type: TokenType.ElseIf;
  exp: string;
  line: number;
  column: number;
  children: Node[];
}

interface ElseNode {
  type: TokenType.Else;

  line: number;
  column: number;
  children: Node[];
}

interface EndIfNode {
  type: TokenType.EndIf;
  line: number;
  column: number;
  children: Node[];
}

interface IscriptNode {
  type: TokenType.Iscript;
  line: number;
  column: number;
  children: Node[];
}

interface EndscriptNode {
  type: TokenType.Endscript;
  line: number;
  column: number;
  children: Node[];
}

interface HtmlNode {
  type: TokenType.Html;
  line: number;
  column: number;
  children: Node[];
}

interface EndhtmlNode {
  type: TokenType.Endhtml;
  line: number;
  column: number;
  children: Node[];
}

interface InlineLanguageNode {
  type: TokenType.InlineLanguage;
  line: number;
  column: number;
  children: Node[];
  sources: string[];
}

//ASTのノードタイプを定義
type Node =
  | TagNode
  | TextNode
  | CommentNode
  | MacroNode
  | IfNode
  | ElseIfNode
  | ElseNode
  | EndIfNode
  | IscriptNode
  | EndscriptNode
  | HtmlNode
  | EndhtmlNode
  | InlineLanguageNode
  | EndMacroNode;

// ParseError クラス
class ParseError extends Error {
  constructor(message: string, public line: number, public column: number) {
    super(message + ` at line ${line}, column ${column}`);
    this.name = "ParseError";
    this.line = line;
    this.column = column;
  }
}

// 	this.line
// パーサー
class TyranoScriptParser {
  private tokens: Token[] = [];
  private currentTokenIndex = 0;
  parse(script: string): Node[] {
    let nodes: Node[] = [];
    const lines = script.split("\n");
    const stack: (
      | { type: TokenType.Macro; nodes: Node[] }
      | { type: TokenType.If; nodes: Node[] }
      | { type: TokenType.ElseIf; nodes: Node[] }
      | { type: TokenType.Else; nodes: Node[] }
      | { type: TokenType.Html; nodes: Node[] }
      | { type: TokenType.Iscript; nodes: Node[] }
    )[] = [];
    let onInlineLanguage = false;

    for (let i = 0; i < lines.length; i++) {
      console.log("line", i, nodes, stack);
      // インライン言語の場合
      if (onInlineLanguage) {
        const line = lines[i];
        switch (stack[stack.length - 1].type) {
          case TokenType.Iscript: {
            if (line.includes("[endscript]")) {
              onInlineLanguage = false;
            } else {
              const inlineLanguageNode: InlineLanguageNode = {
                type: TokenType.InlineLanguage,
                line: i,
                column: 0,
                children: [],
                sources: [line],
              };
              nodes.push(inlineLanguageNode);
              break;
            }
            break;
          }
          case TokenType.Html: {
            if (line.includes("[endhtml]")) {
              onInlineLanguage = false;
            } else {
              const inlineLanguageNode: InlineLanguageNode = {
                type: TokenType.InlineLanguage,
                line: i,
                column: 0,
                children: [],
                sources: [line],
              };
              nodes.push(inlineLanguageNode);
              break;
            }
            break;
          }
        }
      }
      const line = lines[i];
      if (onInlineLanguage) {
        continue;
      }
      let column = 0;
      while (column < line.length) {
        const char = line[column];
        if(char.trim().length === 0){
          column++;
          continue;
        }
        console.log("char", char);

        if (char === "[") {
          let nextColumn: number;
          let node: Node;

          if (stack.length > 0) {
            [node, nextColumn] = this.parseTag(line, i, column);
          } else {
            [node, nextColumn] = this.parseTag(line, i, column);
          }

          //スタックに積むべきノードを判定
          switch (node.type) {
            case TokenType.Iscript:
            case TokenType.Html:
              onInlineLanguage = true;
            // eslint-disable-next-line no-fallthrough
            case TokenType.Macro:
            case TokenType.If: {
              nodes.push(node);
              stack.push({
                type: node.type,
                nodes: nodes,
              });
              nodes = [];
              break;
            }
            case TokenType.ElseIf: {
              const current_stack = stack.pop();
              if (current_stack) {
                if (
                  current_stack.type === TokenType.If ||
                  current_stack.type === TokenType.ElseIf
                ) {
                  current_stack.nodes[current_stack.nodes.length - 1].children =
                    nodes;
                  current_stack.nodes.push(node);
                  stack.push({
                    type: TokenType.ElseIf,
                    nodes: current_stack.nodes,
                  });
                  nodes = [];
                }
              } else {
                throw new ParseError(
                  "elseifがifやelseifの外にあります",
                  i,
                  column
                );
              }
              break;
            }
            case TokenType.Else: {
              const current_stack = stack.pop();
              if (current_stack) {
                if (
                  current_stack.type === TokenType.If ||
                  current_stack.type === TokenType.ElseIf
                ) {
                  current_stack.nodes[current_stack.nodes.length - 1].children =
                    nodes;
                  current_stack.nodes.push(node);
                  stack.push({
                    type: TokenType.Else,
                    nodes: current_stack.nodes,
                  });
                  nodes = [];
                } else {
                  throw new ParseError(
                    "elseがifやelseifの外にあります",
                    i,
                    column
                  );
                }
              }
              break;
            }
            case TokenType.EndIf: {
              const current_stack = stack.pop();
              if (current_stack) {
                if (
                  current_stack.type === TokenType.If ||
                  current_stack.type === TokenType.ElseIf ||
                  current_stack.type === TokenType.Else
                ) {
                  current_stack.nodes[current_stack.nodes.length - 1].children =
                    nodes;
                  current_stack.nodes.push(node);
                  nodes = current_stack.nodes;
                } else {
                  throw new ParseError(
                    "ifが正しく閉じられていません",
                    i,
                    column
                  );
                }
              }
              break;
            }

            case TokenType.EndMacro: {
              const current_stack = stack.pop();
              if (current_stack) {
                if (current_stack.type === TokenType.Macro) {
                  current_stack.nodes[current_stack.nodes.length - 1].children =
                    nodes;
                  current_stack.nodes.push(node);
                  nodes = current_stack.nodes;
                } else {
                  throw new ParseError(
                    "macroが正しく閉じられていません",
                    i,
                    column
                  );
                }
              } else {
                throw new ParseError(
                  "macroが正しく閉じられていません",
                  i,
                  column
                );
              }
              break;
            }
            case TokenType.Endscript: {
              const current_stack = stack.pop();
              if (current_stack) {
                if (current_stack.type === TokenType.Iscript) {
                  current_stack.nodes[current_stack.nodes.length - 1].children =
                    nodes;
                  current_stack.nodes.push(node);
                  nodes = current_stack.nodes;
                } else {
                  throw new ParseError(
                    "iscriptが正しく閉じられていません",
                    i,
                    column
                  );
                }
              }
              onInlineLanguage = false;
              break;
            }
            case TokenType.Endhtml: {
              {
                const current_stack = stack.pop();
                if (current_stack) {
                  if (current_stack.type === TokenType.Html) {
                    current_stack.nodes[
                      current_stack.nodes.length - 1
                    ].children = nodes;
                    current_stack.nodes.push(node);
                    nodes = current_stack.nodes;
                  } else {
                    throw new ParseError(
                      "htmlが正しく閉じられていません",
                      i,
                      column
                    );
                  }
                }
                break;
              }
            }
            default:
              // その他のタグは現在のノードの子ノードとして追加
              nodes.push(node);

              break;
          }

          column = nextColumn; // タグの長さ分だけカラムを進める
        } else if (char === ";") {
          // コメントをパース
          const commentNode: CommentNode = {
            value: line.slice(column).trim().substring(1),
            line: i,
            column: column,
            type: TokenType.Comment,
            children: [],
          };

          nodes.push(commentNode);
          break; // コメント後は改行まで無視
        } else if (char === "/" && line[column + 1] === "*") {
          const [commentNode, nextLine, nextColumn] =
            this.parseMultiLineComment(lines, i, column);
          nodes.push(commentNode);
          i = nextLine;
          column = nextColumn;
        } else {
          // テキストをパース
          const [textNode, nextColumn] = this.parseText(line, i, column);
          if (textNode.value.length > 0) {
            nodes.push(textNode);
          }
          column = nextColumn;
        }
      }
    }

    return nodes;
  }
  private parseTag(
    line: string,
    lineNumber: number,
    column: number
  ): [Node, number] {
    const startIndex = column;
    let tagContent = "";
    let currentColumn = column;

    while (currentColumn < line.length && line[currentColumn] !== "]") {
      tagContent += line[currentColumn];
      currentColumn++;
    }
    if (currentColumn < line.length) {
      tagContent += "]";
      currentColumn++;
    }
    line = line.trim();

    const parameters: TagParameter[] = [];
    const tagParts = tagContent.slice(1, -1).split(/\s+/).filter(Boolean);

    const tagName = tagParts.shift() || "";

    for (const part of tagParts) {
      const [name, value] = part.split("=");

      let paramValue = "";

      if (value) {
        // "" で囲まれた値を抽出
        const match = value.match(/^"(.*)"$/);

        if (match) {
          paramValue = '"' + match[1] + '"';
        } else {
          paramValue = value;
        }
      }

      parameters.push({
        name: name,
        value: paramValue,
        children: [],
      });
    }

    let node: Node;
    let inheritParams = false;
    // macroタグの * を判定
    if (parameters.length > 0 && parameters[0].name === "*") {
      inheritParams = true;
      parameters.shift(); // * パラメータを削除
    }

    if (tagName === "macro") {
      const macroNode: MacroNode = {
        type: TokenType.Macro,
        name: parameters.length > 0 ? parameters[0].value : "",
        line: lineNumber,
        column: startIndex,
        children: [],
      };
      node = macroNode;
    } else if (tagName === "if") {
      const ifNode: IfNode = {
        type: TokenType.If,
        exp: parameters.length > 0 ? parameters[0].value : "",
        line: lineNumber,
        column: startIndex,
        children: [],
      };
      node = ifNode;
    } else if (tagName === "elsif") {
      const elseifNode: ElseIfNode = {
        type: TokenType.ElseIf,
        exp: parameters.length > 0 ? parameters[0].value : "",
        line: lineNumber,
        column: startIndex,
        children: [],
      };
      node = elseifNode;
    } else if (tagName === "else") {
      const elseNode: ElseNode = {
        type: TokenType.Else,
        line: lineNumber,
        column: startIndex,
        children: [],
      };
      node = elseNode;
    } else if (tagName === "endif") {
      const endifNode: EndIfNode = {
        type: TokenType.EndIf,
        line: lineNumber,
        column: startIndex,
        children: [],
      };

      node = endifNode;
    } else if (tagName === "iscript") {
      const iscriptNode: IscriptNode = {
        type: TokenType.Iscript,
        line: lineNumber,
        column: startIndex,
        children: [],
      };
      node = iscriptNode;
    } else if (tagName === "endscript") {
      const endscriptNode: EndscriptNode = {
        type: TokenType.Endscript,
        line: lineNumber,
        column: startIndex,
        children: [],
      };
      node = endscriptNode;
    } else if (tagName === "html") {
      const htmlNode: HtmlNode = {
        type: TokenType.Html,
        line: lineNumber,
        column: startIndex,
        children: [],
      };

      node = htmlNode;
    } else if (tagName === "endhtml") {
      const endhtmlNode: EndhtmlNode = {
        type: TokenType.Endhtml,
        line: lineNumber,
        column: startIndex,
        children: [],
      };
      node = endhtmlNode;
    } else if (tagName === "endmacro") {
      const endMacroNode: EndMacroNode = {
        type: TokenType.EndMacro,
        line: lineNumber,
        column: startIndex,
        children: [],
      };
      node = endMacroNode;
    } else {
      const tagNode: TagNode = {
        name: tagName,
        parameters: parameters,
        line: lineNumber,
        column: startIndex,
        type: TokenType.Tag,
        inheritParams: inheritParams, // inheritParams を設定
        children: [],
      };
      node = tagNode;
    }

    return [node, currentColumn];
  }

  private parseMultiLineComment(
    lines: string[],
    lineNumber: number,
    column: number
  ): [CommentNode, number, number] {
    let commentContent = "";
    let i = lineNumber;
    let currentColumn = column;
    let endComment = false;

    while (i < lines.length && !endComment) {
      const line = lines[i];

      while (currentColumn < line.length && !endComment) {
        if (line.slice(currentColumn, currentColumn + 2) === "*/") {
          commentContent += "*/";
          currentColumn += 2;

          endComment = true;
        } else {
          commentContent += line[currentColumn].trim();
          currentColumn++;
        }
      }
      if (!endComment) {
        commentContent += "\n";
        i++;
        currentColumn = 0;
      }
    }
    if (!endComment) {
      // 複数行コメントが閉じられていない場合のエラー処理
      console.error("Error: 複数行コメントが閉じられていません");
      //ダミー値を返す.
      const dummyNode: CommentNode = {
        value: commentContent,
        line: -1,
        column: -1,
        type: TokenType.Comment,
        children: [],
      };
      return [dummyNode, -1, -1];
    }

    const commentNode: CommentNode = {
      value: commentContent,
      line: lineNumber,
      column: column,
      type: TokenType.Comment,
      children: [],
    };

    return [commentNode, i, currentColumn];
  }

  private parseText(
    line: string,
    lineNumber: number,
    column: number
  ): [TextNode, number] {
    let textContent = "";
    let currentColumn = column;

    while (currentColumn < line.length) {
      const char = line[currentColumn];
      if (char === "[") {
        throw new ParseError(
          "実装ミス: タグの開始が見つかりました",
          lineNumber,
          currentColumn
        );
        break; // タグの開始なので終了
      } else if (char === ";") {
        throw new ParseError(
          "実装ミス: コメントの開始が見つかりました",
          lineNumber,
          currentColumn
        );
        break;
      } else if (char === "/" && line[currentColumn + 1] === "*") {
        throw new ParseError(
          "実装ミス: 複数行コメントの開始が見つかりました",
          lineNumber,
          currentColumn
        );
        break;
      } else {
        textContent += char;
      }
      currentColumn++;
    }
    let node: TextNode;
    if (textContent.trim().length === 0) {
      node = {
        value: "",
        line: -1,
        column: -1,
        type: TokenType.Text,
        children: [],
      };
    } else {
      node = {
        value: textContent,
        line: lineNumber,
        column: column,
        type: TokenType.Text,
        children: [],
      };
    }

    return [node, currentColumn];
  }
}

export {
  TyranoScriptParser,
  Token,
  TokenType,
  Node,
  TagParameter,
  TagNode,
  TextNode,
  CommentNode,
  MacroNode,
  IfNode,
  ElseIfNode,
  ElseNode,
  EndIfNode,
  IscriptNode,
  EndscriptNode,
  HtmlNode,
  EndhtmlNode,
  ParseError,
};
