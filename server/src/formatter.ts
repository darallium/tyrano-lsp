import {
  Node,
  TagNode,
  TextNode,
  CommentNode,
  MacroNode,
  TokenType,
} from "./parser";

// Formatterの設定
export interface FormatterOptions {
  indentSize: number;
  useTabs: boolean;
  tagAttributesOnNewLine: boolean;
}

export class TyranoScriptFormatter {
  private options: FormatterOptions;

  constructor(options: FormatterOptions) {
    this.options = options;
  }

  format(nodes: Node[]): string {
    let formattedScript = "";
    const indentLevel = 0;

    for (const node of nodes) {
      if (node.type === TokenType.Tag || node.type === TokenType.Macro) {
        formattedScript += this.formatTag(
          node as TagNode | MacroNode,
          indentLevel
        );
      } else if (node.type === TokenType.Text) {
        formattedScript += this.formatText(node as TextNode, indentLevel);
      } else if (node.type === TokenType.Comment) {
        formattedScript += this.formatComment(node as CommentNode, indentLevel);
      }
    }

    return formattedScript;
  }

  private formatTag(node: TagNode | MacroNode, indentLevel: number): string {
    const indent = this.getIndent(indentLevel);
    let formattedTag = `${indent}[${node.name}`;

    if (node.parameters.length > 0) {
      if (this.options.tagAttributesOnNewLine) {
        formattedTag += "\n";
        for (const param of node.parameters) {
          formattedTag += `${this.getIndent(indentLevel + 1)}${param.name}="${
            param.value
          }"\n`;
        }
        formattedTag += indent;
      } else {
        for (const param of node.parameters) {
          formattedTag += ` ${param.name}="${param.value}"`;
        }
      }
    }

    formattedTag += "]";
    // macroタグの場合は * を付加
    if ("inheritParams" in node && node.inheritParams) {
      formattedTag = formattedTag.replace("[macro", "[macro *");
    }

    formattedTag += "\n";

    return formattedTag;
  }

  private formatText(node: TextNode, indentLevel: number): string {
    // テキストノードを整形
    const indent = this.getIndent(indentLevel);
    const formattedLines: string[] = [];
    node.value.split("\n").forEach((line) => {
      formattedLines.push(indent + line);
    });

    return formattedLines.join("\n") + "\n";
  }
  private formatComment(node: CommentNode, indentLevel: number): string {
    // コメントノードの場合はインデントを追加してそのまま返す
    const indent = this.getIndent(indentLevel);

    return indent + node.value + "\n";
  }
  private getIndent(level: number): string {
    const indentUnit = this.options.useTabs
      ? "\t"
      : " ".repeat(this.options.indentSize);

    return indentUnit.repeat(level);
  }
}
