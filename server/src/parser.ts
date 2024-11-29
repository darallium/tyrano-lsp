// トークンの種類
enum TokenType {
  Tag,
  Text,
  Comment,
  Macro,
  If,
  ElseIf,
  Else,
  EndIf,
  Iscript,
  Endscript,
  Html,
  Endhtml,
}

// トークン
interface Token {
  type: TokenType;
  value: string;
  line: number;
  column: number;
}

// タグのパラメータ
interface TagParameter {
  name: string;
  value: string;
}

// タグのASTノード
interface TagNode {
  type: TokenType.Tag;
  name: string;
  parameters: TagParameter[];
  line: number;
  column: number;
}

// テキストのASTノード
interface TextNode {
  type: TokenType.Text;
  value: string;
  line: number;
  column: number;
}
// コメントのASTノード
interface CommentNode {
  type: TokenType.Comment;
  value: string;
  line: number;
  column: number;
}

// マクロ呼び出しノード
interface MacroNode {
  type: TokenType.Macro;
  name: string;
  parameters: TagParameter[];
  line: number;
  column: number;
  inheritParams?: boolean; // *付きかどうかを示すフラグ
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
}

interface IscriptNode {
  type: TokenType.Iscript;
  line: number;
  column: number;
  children: Node[]; // 子ノードを追加
}

interface EndscriptNode {
  type: TokenType.Endscript;
  line: number;
  column: number;
}

interface HtmlNode {
  type: TokenType.Html;
  line: number;
  column: number;
  children: Node[]; // 子ノードを追加
}

interface EndhtmlNode {
  type: TokenType.Endhtml;
  line: number;
  column: number;
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
  | EndhtmlNode;

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
    const nodes: Node[] = [];
    const lines = script.split("\n");
    // let stack: (
    //   | { type: TokenType.Macro; node: MacroNode }
    //   | { type: TokenType.If; node: IfNode }
    //   | { type: TokenType.ElseIf; node: ElseIfNode }
    //   | { type: TokenType.Else; node: ElseNode }
    //   | { type: TokenType.Html; node: HtmlNode }
    //   | { type: TokenType.Iscript; node: IscriptNode }
    // )[] = [];

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const stack: { type: TokenType; node: any }[] = [];

    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      let column = 0;
      while (column < line.length) {
        const char = line[column];

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
            case TokenType.Macro: {
              stack.push({
                type: TokenType.Macro,
                node: node,
              });
              break;
            }
            case TokenType.Iscript:
            case TokenType.Html:
              stack.push({
                type: node.type,
                node: node,
              });
              break;
            case TokenType.If: {
              stack.push({
                type: TokenType.If,
                node: node,
              });
              break;
            }
            case TokenType.ElseIf: {
              const current_stack = stack.pop();
              if (current_stack) {
                if (
                  current_stack.type === TokenType.If ||
                  current_stack.type === TokenType.ElseIf
                ) {
                  current_stack.node.children = nodes;
                  stack.push({ type: TokenType.ElseIf, node: node });
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
                  current_stack.node.children = nodes;
                  stack.push({ type: TokenType.Else, node: node });
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
            case TokenType.EndIf:
            case TokenType.Endscript:
            case TokenType.Endhtml: {
              const current_endif_stack = stack.pop();
              if (current_endif_stack) {
                current_endif_stack.node.children = nodes;
                if (stack.length > 0) {
                  const parent = stack[stack.length - 1].node;
                  parent.children.push(current_endif_stack.node);
                } else {
                  nodes.push(current_endif_stack.node);
                }
              }
              //nodes = [];
              break;
            }
            default:
              // その他のタグは現在のノードの子ノードとして追加
              if (stack.length > 0) {
                stack[stack.length - 1].node.children.push(node);
              } else {
                nodes.push(node);
              }

              break;
          }

          column = nextColumn; // タグの長さ分だけカラムを進める
        } else if (char === ";") {
          // コメントをパース
          const commentNode: CommentNode = {
            value: line.slice(column),
            line: i,
            column: column,
            type: TokenType.Comment,
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
          nodes.push(textNode);
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
          paramValue = match[1];
        } else {
          paramValue = value;
        }
      }

      parameters.push({ name: name, value: paramValue });
    }

    let node: Node;
    let inheritParams = false;
    // macroタグの * を判定
    if (
      tagName === "macro" &&
      parameters.length > 0 &&
      parameters[0].name === "*"
    ) {
      inheritParams = true;
      parameters.shift(); // * パラメータを削除
    }

    if (tagName === "macro") {
      const macroNode: MacroNode = {
        type: TokenType.Macro,
        name: tagName,
        parameters: parameters,
        line: lineNumber,
        column: startIndex,
        inheritParams: inheritParams, // inheritParams を設定
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
      };
      node = endhtmlNode;
    } else {
      const tagNode: TagNode = {
        name: tagName,
        parameters: parameters,
        line: lineNumber,
        column: startIndex,
        type: TokenType.Tag,
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
          commentContent += line[currentColumn];
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
      };
      return [dummyNode, -1, -1];
    }

    const commentNode: CommentNode = {
      value: commentContent,
      line: lineNumber,
      column: column,
      type: TokenType.Comment,
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
        break; // タグの開始なので終了
      } else if (char === ";") {
        break;
      } else if (char === "/" && line[currentColumn + 1] === "*") {
        break;
      } else {
        textContent += char;
      }
      currentColumn++;
    }

    const textNode: TextNode = {
      value: textContent,
      line: lineNumber,
      column: column,
      type: TokenType.Text,
    };

    return [textNode, currentColumn];
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
