use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

fn empty_schema() -> serde_json::Value {
    serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "additionalProperties": false
    })
}

fn file_path_schema() -> serde_json::Value {
    serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "required": ["filePath"],
        "additionalProperties": false,
        "properties": {
            "filePath": { "type": "string", "description": "Path to the file" }
        }
    })
}

pub fn tool_list() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "getCurrentSelection",
            description: "Get the current text selection in the editor",
            input_schema: empty_schema(),
        },
        ToolDef {
            name: "getLatestSelection",
            description: "Get the most recent text selection (even if not in the active editor)",
            input_schema: empty_schema(),
        },
        ToolDef {
            name: "getOpenEditors",
            description: "Get list of currently open files",
            input_schema: empty_schema(),
        },
        ToolDef {
            name: "getWorkspaceFolders",
            description: "Get all workspace folders currently open in the IDE",
            input_schema: empty_schema(),
        },
        ToolDef {
            name: "openFile",
            description: "Open a file in the editor and optionally select a range of text",
            input_schema: serde_json::json!({
                "$schema": "http://json-schema.org/draft-07/schema#",
                "type": "object",
                "required": ["filePath"],
                "additionalProperties": false,
                "properties": {
                    "filePath": { "type": "string", "description": "Path to the file to open" },
                    "preview": { "type": "boolean", "description": "Open in preview mode", "default": false },
                    "startLine": { "type": "integer", "description": "Start line of selection" },
                    "endLine": { "type": "integer", "description": "End line of selection" },
                    "startText": { "type": "string", "description": "Text pattern to find selection start" },
                    "endText": { "type": "string", "description": "Text pattern to find selection end" },
                    "selectToEndOfLine": { "type": "boolean", "description": "Extend selection to end of line", "default": false },
                    "makeFrontmost": { "type": "boolean", "description": "Make file the active tab", "default": true }
                }
            }),
        },
        ToolDef {
            name: "openDiff",
            description: "Open a diff view comparing old file content with new file content",
            input_schema: serde_json::json!({
                "$schema": "http://json-schema.org/draft-07/schema#",
                "type": "object",
                "required": ["old_file_path", "new_file_path", "new_file_contents", "tab_name"],
                "additionalProperties": false,
                "properties": {
                    "old_file_path": { "type": "string", "description": "Path to the old file" },
                    "new_file_path": { "type": "string", "description": "Path to the new file" },
                    "new_file_contents": { "type": "string", "description": "Contents for the new file version" },
                    "tab_name": { "type": "string", "description": "Name for the diff tab/view" }
                }
            }),
        },
        ToolDef {
            name: "checkDocumentDirty",
            description: "Check if a document has unsaved changes",
            input_schema: file_path_schema(),
        },
        ToolDef {
            name: "saveDocument",
            description: "Save a document with unsaved changes",
            input_schema: file_path_schema(),
        },
        ToolDef {
            name: "closeAllDiffTabs",
            description: "Close all diff tabs in the editor",
            input_schema: empty_schema(),
        },
        ToolDef {
            name: "getDiagnostics",
            description: "Get language diagnostics (errors, warnings) from the editor",
            input_schema: serde_json::json!({
                "$schema": "http://json-schema.org/draft-07/schema#",
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "uri": { "type": "string", "description": "Optional file URI to get diagnostics for" }
                }
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_list_has_all_tools() {
        let tools = tool_list();
        let names: Vec<&str> = tools.iter().map(|t| t.name).collect();
        assert!(names.contains(&"getCurrentSelection"));
        assert!(names.contains(&"getLatestSelection"));
        assert!(names.contains(&"getOpenEditors"));
        assert!(names.contains(&"getWorkspaceFolders"));
        assert!(names.contains(&"openFile"));
        assert!(names.contains(&"openDiff"));
        assert!(names.contains(&"checkDocumentDirty"));
        assert!(names.contains(&"saveDocument"));
        assert!(names.contains(&"closeAllDiffTabs"));
        assert!(names.contains(&"getDiagnostics"));
        assert_eq!(names.len(), 10);
    }

    #[test]
    fn test_tool_list_serializes() {
        let tools = tool_list();
        let json = serde_json::to_value(&tools).unwrap();
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 10);
        // Each tool should have name, description, inputSchema
        for tool in arr {
            assert!(tool["name"].is_string());
            assert!(tool["description"].is_string());
            assert!(tool["inputSchema"].is_object());
        }
    }

    #[test]
    fn test_open_file_schema_has_additional_properties_false() {
        let tools = tool_list();
        let open_file = tools.iter().find(|t| t.name == "openFile").unwrap();
        let schema = &open_file.input_schema;
        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["required"].as_array().unwrap().contains(&serde_json::json!("filePath")));
    }
}
