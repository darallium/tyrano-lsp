use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AstNode {
    Scenario {
        lines: Vec<Box<AstNode>>,
    },
    CharacterName {
        name: String,
        face: Option<String>,
    },
    Label {
        name: String,
        text: Option<String>,
    },
    Tag {
        name: String,
        parameters: Vec<Parameter>,
        is_at_notation: bool,
    },
    Text {
        content: String,
        preserve_whitespace: bool,
    },
    Comment {
        content: String,
        is_block: bool,
    },
    Script {
        content: String,
    },
    Html {
        content: String,
    },
    /// Intermediate node produced while reducing a tag's parameter list;
    /// consumed into `Tag::parameters` by the `tag_content` reduction.
    TagParameter {
        name: String,
        value: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub value: Option<String>,
}

impl AstNode {
    pub fn new_scenario(lines: Vec<Box<AstNode>>) -> Self {
        AstNode::Scenario { lines }
    }

    pub fn new_character(name: String, face: Option<String>) -> Self {
        AstNode::CharacterName { name, face }
    }

    pub fn new_label(name: String, text: Option<String>) -> Self {
        AstNode::Label { name, text }
    }

    pub fn new_tag(name: String, parameters: Vec<Parameter>, is_at: bool) -> Self {
        AstNode::Tag {
            name,
            parameters,
            is_at_notation: is_at,
        }
    }

    pub fn new_text(content: String, preserve_whitespace: bool) -> Self {
        AstNode::Text {
            content,
            preserve_whitespace,
        }
    }

    pub fn new_comment(content: String, is_block: bool) -> Self {
        AstNode::Comment { content, is_block }
    }

    // Helper methods for extracting content from AST nodes
    pub fn as_text(&self) -> Option<&str> {
        match self {
            AstNode::Text { content, .. } => Some(content),
            _ => None,
        }
    }

    pub fn into_text(self) -> Option<String> {
        match self {
            AstNode::Text { content, .. } => Some(content),
            _ => None,
        }
    }

    pub fn flatten_text(&self) -> String {
        match self {
            AstNode::Text { content, .. } => content.clone(),
            AstNode::Scenario { lines } => lines
                .iter()
                .map(|child| child.flatten_text())
                .collect::<Vec<_>>()
                .join(""),
            _ => String::new(),
        }
    }

    /// Format the AST as a tree structure
    pub fn format_tree(&self) -> String {
        let mut output = String::new();
        self.format_tree_internal(&mut output, "", true);
        output
    }

    fn format_tree_internal(&self, output: &mut String, prefix: &str, is_last: bool) {
        // Current node connector
        let connector = if prefix.is_empty() {
            ""
        } else if is_last {
            "└── "
        } else {
            "├── "
        };

        // Node description
        let node_desc = match self {
            AstNode::Scenario { lines } => {
                output.push_str(&format!(
                    "{}{}Scenario ({} lines)\n",
                    prefix,
                    connector,
                    lines.len()
                ));

                // Process children
                let child_prefix = if prefix.is_empty() {
                    String::new()
                } else if is_last {
                    format!("{}    ", prefix)
                } else {
                    format!("{}│   ", prefix)
                };

                for (i, child) in lines.iter().enumerate() {
                    let is_last_child = i == lines.len() - 1;
                    child.format_tree_internal(output, &child_prefix, is_last_child);
                }
                return;
            }
            AstNode::CharacterName { name, face } => {
                if let Some(face_value) = face {
                    format!(
                        "{}{}CharacterName: \"{}\" (face: \"{}\")\n",
                        prefix, connector, name, face_value
                    )
                } else {
                    format!("{}{}CharacterName: \"{}\"\n", prefix, connector, name)
                }
            }
            AstNode::Label { name, text } => {
                if let Some(text_value) = text {
                    format!(
                        "{}{}Label: \"{}\" (text: \"{}\")\n",
                        prefix, connector, name, text_value
                    )
                } else {
                    format!("{}{}Label: \"{}\"\n", prefix, connector, name)
                }
            }
            AstNode::Tag {
                name,
                parameters,
                is_at_notation,
            } => {
                let notation = if *is_at_notation { "@" } else { "" };
                output.push_str(&format!(
                    "{}{}Tag: {}{} ({} parameters)\n",
                    prefix,
                    connector,
                    notation,
                    name,
                    parameters.len()
                ));

                if !parameters.is_empty() {
                    let param_prefix = if prefix.is_empty() {
                        String::new()
                    } else if is_last {
                        format!("{}    ", prefix)
                    } else {
                        format!("{}│   ", prefix)
                    };

                    for (i, param) in parameters.iter().enumerate() {
                        let is_last_param = i == parameters.len() - 1;
                        let param_connector = if is_last_param {
                            "└── "
                        } else {
                            "├── "
                        };

                        if let Some(value) = &param.value {
                            output.push_str(&format!(
                                "{}{}Parameter: {}=\"{}\"\n",
                                param_prefix, param_connector, param.name, value
                            ));
                        } else {
                            output.push_str(&format!(
                                "{}{}Parameter: {}\n",
                                param_prefix, param_connector, param.name
                            ));
                        }
                    }
                }
                return;
            }
            AstNode::Text {
                content,
                preserve_whitespace,
            } => {
                let preview = preview_of(content);
                let ws_flag = if *preserve_whitespace {
                    " [preserve-ws]"
                } else {
                    ""
                };
                format!("{}{}Text: \"{}\"{}\n", prefix, connector, preview, ws_flag)
            }
            AstNode::Comment { content, is_block } => {
                let comment_type = if *is_block { "block" } else { "line" };
                let preview = preview_of(content);
                format!(
                    "{}{}Comment ({}): \"{}\"\n",
                    prefix, connector, comment_type, preview
                )
            }
            AstNode::Script { content } => {
                let preview = preview_of(content);
                format!("{}{}Script: \"{}\"\n", prefix, connector, preview)
            }
            AstNode::Html { content } => {
                let preview = preview_of(content);
                format!("{}{}Html: \"{}\"\n", prefix, connector, preview)
            }
            AstNode::TagParameter { name, value } => {
                if let Some(value) = value {
                    format!("{}{}Parameter: {}=\"{}\"\n", prefix, connector, name, value)
                } else {
                    format!("{}{}Parameter: {}\n", prefix, connector, name)
                }
            }
        };

        output.push_str(&node_desc);
    }
}

/// Shorten long content for tree display, safe on multibyte text.
fn preview_of(content: &str) -> String {
    if content.chars().count() > 50 {
        let head: String = content.chars().take(47).collect();
        format!("{}...", head)
    } else {
        content.to_string()
    }
}
