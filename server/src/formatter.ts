import { Node, TokenType } from "./parser";

// Formatterの設定
export interface FormatterOptions {
  indentSize: number;
  useTabs: boolean;
  tagAttributesOnNewLine: boolean;
  newline: string; // 追加: 改行コードのオプション
}

export class TyranoScriptFormatter {
  private options: FormatterOptions;

  constructor(options: FormatterOptions) {
    this.options = options;
  }

  format(nodes: Node[]): string {
    let formattedScript = "";
    const indentLevel = 0;
    formattedScript += this.formatChildren(nodes, indentLevel);
    return formattedScript;
  }
  private formatChildren(nodes: Node[], indentLevel: number): string {
    let formattedScript = "";
    for (const node of nodes) {
      formattedScript += this.formatTag(node, indentLevel);
      if (node.children) {
        formattedScript += this.formatChildren(node.children, indentLevel + 1);
      }
    }
    return formattedScript;
  }

  private formatTag(node: Node, indentLevel: number): string {
    const indent = this.getIndent(indentLevel);
    let formattedTag = "";
    const newline = this.options.newline; // 追加: 改行コードのオプションを使用

    switch (node.type) {
      case TokenType.Comment: {
        formattedTag = `${indent}; ${node.value.trim()}${newline}`;
        break;
      }
      case TokenType.Text: {
        const formattedLines: string[] = [];
        node.value.split("\n").forEach((line) => {
          formattedLines.push(indent + line.trim());
        });
        formattedTag = formattedLines.join(newline) + newline;
        break;
      }
      case TokenType.InlineLanguage: {
        const formattedLines: string[] = [];
        node.sources.forEach((line) => {
          formattedLines.push(line);
        });
        formattedTag = formattedLines.join(newline) + newline;
        break;
      }
      case TokenType.If: {
        formattedTag = `${indent}[if exp=${node.exp}]${newline}`;
        break;
      }
      case TokenType.ElseIf: {
        formattedTag = `${indent}[elsif exp=${node.exp}]${newline}`;
        break;
      }
      case TokenType.Else: {
        formattedTag = `${indent}[else]${newline}`;
        break;
      }
      case TokenType.EndIf: {
        formattedTag = `${indent}[endif]${newline}`;
        break;
      }
      case TokenType.Macro: {
        formattedTag = `${indent}[macro name=${node.name}]${newline}`;
        break;
      }
      case TokenType.Iscript: {
        formattedTag = `${indent}[iscript]${newline}`;
        break;
      }
      case TokenType.Endscript: {
        formattedTag = `${indent}[endscript]${newline}`;
        break;
      }
      case TokenType.Html: {
        formattedTag = `${indent}[html]${newline}`;
        // TODO: FIXME
        break;
      }
      case TokenType.Endhtml: {
        formattedTag = `${indent}[endhtml]${newline}`;
        break;
      }
      case TokenType.Tag: {
        formattedTag = `${indent}[${node.name}`;
        if (node.parameters.length > 0) {
          for (const param of node.parameters) {
            formattedTag += ` ${param.name}=${param.value}`;
          }
        }
        formattedTag += `]${newline}`;
        break;
      }
      case TokenType.EndMacro: {
        formattedTag = `${indent}[endmacro]${newline}`;
        break;
      }
      default: {
        throw new Error(`Unknown node type: ${node}`);
      }
    }

    return formattedTag;
  }

  private getIndent(level: number): string {
    const indentUnit = this.options.useTabs
      ? "\t"
      : " ".repeat(this.options.indentSize);

    return indentUnit.repeat(level);
  }
}
