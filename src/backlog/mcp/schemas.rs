//! Tool definitions and JSON schemas for `bencode --backlog-mcp`, grouped by
//! [`Toolset`] so `--toolsets` can filter what an agent sees.

use serde_json::{Value, json};

use super::Toolset;

pub(super) fn all_tools() -> Vec<(Toolset, Value)> {
    let key = json!({
        "type": "string",
        "description": "The issue's key (PROJ-123) or its Backlog address.",
    });
    let project_key = json!({
        "type": "string",
        "description": "The project's key (PROJ) or numeric id.",
    });
    let name_or_id = |what: &str| {
        json!({
            "type": ["string", "integer", "null"],
            "description": format!("{what}: its name or id; null clears it."),
        })
    };
    let day = |what: &str| {
        json!({
            "type": ["string", "null"],
            "description": format!("{what}, YYYY-MM-DD; null clears it."),
        })
    };
    let filter_day = |what: &str| {
        json!({
            "type": "string",
            "description": format!("{what} (YYYY-MM-DD)."),
        })
    };
    let hours = |what: &str| {
        json!({
            "type": ["number", "null"],
            "description": format!("{what}; null clears them."),
        })
    };
    let assignee = json!({
        "type": ["string", "integer", "null"],
        "description": "The assignee: \"me\", a member's name or id; null clears it.",
    });
    let user_list = |what: &str| {
        json!({
            "type": "array",
            "items": { "type": ["string", "integer"] },
            "description": format!("{what}: \"me\", member names or ids."),
        })
    };
    let named_list = |what: &str| {
        json!({
            "type": "array",
            "items": { "type": ["string", "integer"] },
            "description": format!("{what}, by name or id; [] clears them."),
        })
    };
    let custom_fields_write = json!({
        "type": "array",
        "description": "Custom field values to set. Identify each field by numeric id or exact name.",
        "items": {
            "type": "object",
            "properties": {
                "id": { "type": "integer", "description": "Custom field id (from backlog_get_project)." },
                "name": { "type": "string", "description": "Custom field name (alternative to id)." },
                "value": {
                    "description": "String, number, item name/id, array of item names/ids, or null to clear.",
                },
                "otherValue": { "type": "string", "description": "Other text value for checkbox/radio fields." },
            },
        },
    });
    let repo = json!({
        "type": ["string", "integer"],
        "description": "The Git repository's name or numeric id.",
    });

    let mut search_props = serde_json::Map::new();
    for (k, v) in [
        ("project", json!({ "type": "string", "description": "A project key or id; every active project when left out." })),
        ("keyword", json!({ "type": "string", "description": "Words in the subject, description or comments." })),
        ("assignedToMe", json!({ "type": "boolean" })),
        ("assignee", json!({ "type": ["string", "integer", "array"], "description": "\"me\", member name/id, or array of them." })),
        ("createdBy", json!({ "type": ["string", "integer", "array"], "description": "Creator: \"me\", member name/id, or array of them." })),
        ("includeClosed", json!({ "type": "boolean" })),
        ("statuses", json!({ "type": "array", "items": { "type": ["string", "integer"] }, "description": "Filter by status names or ids (overrides default open-only filter)." })),
        ("issueTypes", json!({ "type": "array", "items": { "type": ["string", "integer"] }, "description": "Filter by issue type names or ids (requires project for names)." })),
        ("categories", json!({ "type": "array", "items": { "type": ["string", "integer"] }, "description": "Filter by category names or ids (requires project for names)." })),
        ("milestones", json!({ "type": "array", "items": { "type": ["string", "integer"] }, "description": "Filter by milestone names or ids (requires project for names)." })),
        ("versions", json!({ "type": "array", "items": { "type": ["string", "integer"] }, "description": "Filter by version names or ids (requires project for names)." })),
        ("priorities", json!({ "type": "array", "items": { "type": ["string", "integer"] }, "description": "Filter by priority names or ids." })),
        ("resolutions", json!({ "type": "array", "items": { "type": ["string", "integer"] }, "description": "Filter by resolution names or ids." })),
        ("parentIssue", json!({ "type": ["string", "integer"], "description": "Find child issues of this parent issue key, URL or numeric id." })),
        ("parentChild", json!({ "type": "string", "enum": ["all", "not_child", "child", "standalone", "parent"], "description": "Filter by parent/child hierarchy." })),
        ("dueDateSince", filter_day("Due date on or after")),
        ("dueDateUntil", filter_day("Due date on or before")),
        ("startDateSince", filter_day("Start date on or after")),
        ("startDateUntil", filter_day("Start date on or before")),
        ("createdSince", filter_day("Created on or after")),
        ("createdUntil", filter_day("Created on or before")),
        ("updatedSince", filter_day("Updated on or after")),
        ("updatedUntil", filter_day("Updated on or before")),
        ("sort", json!({ "type": "string", "enum": ["updated", "created", "dueDate", "startDate", "priority", "status", "assignee", "issueType", "summary", "estimatedHours", "actualHours", "category", "milestone", "version"] })),
        ("order", json!({ "type": "string", "enum": ["asc", "desc"] })),
        ("offset", json!({ "type": "integer", "description": "Pagination offset (default 0)." })),
        ("limit", json!({ "type": "integer", "description": "At most this many (default 20, at most 100)." })),
        ("countOnly", json!({ "type": "boolean", "description": "Return only the total count of matching issues." })),
        ("customFields", json!({
            "type": "array",
            "description": "Custom field filters: { id, type: 'text'|'numeric'|'date'|'list', value?, min?, max? }.",
            "items": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer" },
                    "type": { "type": "string", "enum": ["text", "numeric", "date", "list"] },
                    "value": {},
                    "min": {},
                    "max": {},
                },
                "required": ["id", "type"],
            },
        })),
    ] {
        search_props.insert(k.to_string(), v);
    }

    vec![
        // -------------------------------------------------------------------
        // Space
        // -------------------------------------------------------------------
        (
            Toolset::Space,
            json!({
                "name": "backlog_get_space",
                "description": "Space info and the connected user; optionally lists all space members (includeUsers) or recent activities (activities).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "includeUsers": { "type": "boolean", "description": "Include all users in the space." },
                        "activities": { "type": "boolean", "description": "Include recent space activities (or one user's when user is set)." },
                        "user": { "type": ["string", "integer"], "description": "Filter activities to one user (\"me\", name or id)." },
                        "limit": { "type": "integer", "description": "Max activities to return (default 20, max 100)." },
                        "minId": { "type": "integer", "description": "Minimum activity id." },
                        "maxId": { "type": "integer", "description": "Maximum activity id." },
                        "order": { "type": "string", "enum": ["asc", "desc"] },
                    },
                },
            }),
        ),
        // -------------------------------------------------------------------
        // Project
        // -------------------------------------------------------------------
        (
            Toolset::Project,
            json!({
                "name": "backlog_list_projects",
                "description": "The space's active projects: each key, id, name and workflow statuses.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "includeArchived": { "type": "boolean", "description": "Also include archived projects." },
                    },
                },
            }),
        ),
        (
            Toolset::Project,
            json!({
                "name": "backlog_get_project",
                "description": "Full metadata for one project: settings, members, statuses, issue types, categories, milestones/versions, custom fields, priorities and resolutions.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": project_key,
                        "includeArchivedMilestones": { "type": "boolean", "description": "Include archived versions/milestones." },
                    },
                    "required": ["project"],
                },
            }),
        ),
        (
            Toolset::Project,
            json!({
                "name": "backlog_create_category",
                "description": "Create a new category in a project. Only when the user asked for it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": project_key,
                        "name": { "type": "string", "description": "The category name." },
                    },
                    "required": ["project", "name"],
                },
            }),
        ),
        (
            Toolset::Project,
            json!({
                "name": "backlog_manage_milestone",
                "description": "Create, update or delete a version/milestone in a project. Only when the user asked for it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": project_key,
                        "action": { "type": "string", "enum": ["create", "update", "delete"] },
                        "milestone": { "type": ["string", "integer"], "description": "Existing milestone name or id (required for update/delete)." },
                        "name": { "type": "string", "description": "Milestone name (required for create; optional rename for update)." },
                        "description": { "type": ["string", "null"] },
                        "startDate": day("Start date"),
                        "releaseDueDate": day("Release due date"),
                        "archived": { "type": "boolean", "description": "Archive or unarchive (update only)." },
                    },
                    "required": ["project", "action"],
                },
            }),
        ),
        (
            Toolset::Project,
            json!({
                "name": "backlog_manage_project",
                "description": "Create a new project or update an existing project's settings. Only when the user asked for it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["create", "update"] },
                        "project": { "type": "string", "description": "Existing project key or id (required for update)." },
                        "name": { "type": "string", "description": "Project display name." },
                        "key": { "type": "string", "description": "Project key (uppercase letters, digits, underscore)." },
                        "chartEnabled": { "type": "boolean" },
                        "subtaskingEnabled": { "type": "boolean" },
                        "grandchildIssueEnabled": { "type": "boolean" },
                        "projectLeaderCanEditProjectLeader": { "type": "boolean" },
                        "textFormattingRule": { "type": "string", "enum": ["markdown", "backlog"] },
                        "archived": { "type": "boolean", "description": "Archive or unarchive (update only)." },
                    },
                    "required": ["action"],
                },
            }),
        ),
        // -------------------------------------------------------------------
        // Issue
        // -------------------------------------------------------------------
        (
            Toolset::Issue,
            json!({
                "name": "backlog_search_issues",
                "description": "Find or count issues with rich filters (status, assignee, type, category, milestone, version, priority, parent/child, dates, custom fields, keyword, sort, offset). Open issues only unless includeClosed or statuses is set.",
                "inputSchema": {
                    "type": "object",
                    "properties": Value::Object(search_props),
                },
            }),
        ),
        (
            Toolset::Issue,
            json!({
                "name": "backlog_get_issue",
                "description": "An issue's subject, fields, custom fields, attachments list and description; its inline pictures are saved as local files the description links. comments: true adds comments (with #commentId) and changes; relatedIssues: true adds related issues.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "key": key,
                        "comments": { "type": "boolean" },
                        "commentOrder": { "type": "string", "enum": ["asc", "desc"], "description": "Comment fetch order when comments is true (default desc, displayed oldest first)." },
                        "commentCount": { "type": "integer", "description": "Max comments to fetch (default 100, max 100)." },
                        "commentMinId": { "type": "integer" },
                        "commentMaxId": { "type": "integer" },
                        "relatedIssues": { "type": "boolean", "description": "Also list related issues." },
                    },
                    "required": ["key"],
                },
            }),
        ),
        (
            Toolset::Issue,
            json!({
                "name": "backlog_create_issue",
                "description": "Create an issue as the user; it starts in the project's first status. Only when the user asked for it. A name that does not match lists the choices.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": project_key,
                        "summary": { "type": "string", "description": "The subject." },
                        "description": { "type": "string", "description": "Backlog's Markdown or plain text." },
                        "issueType": {
                            "type": ["string", "integer"],
                            "description": "The issue type: its name or id; the project's first type when left out.",
                        },
                        "assignee": assignee,
                        "priority": { "type": ["string", "integer"], "description": "The priority: its name or id; Normal when left out." },
                        "categories": named_list("Categories"),
                        "versions": named_list("Affected versions"),
                        "milestones": named_list("Milestones"),
                        "parentIssue": { "type": ["string", "integer"], "description": "Parent issue key, URL or numeric id (for subtasks)." },
                        "startDate": day("The start date"),
                        "dueDate": day("The due date"),
                        "estimatedHours": hours("Estimated hours"),
                        "actualHours": hours("Actual hours"),
                        "notifiedUsers": user_list("Users to notify"),
                        "customFields": custom_fields_write,
                    },
                    "required": ["project", "summary"],
                },
            }),
        ),
        (
            Toolset::Issue,
            json!({
                "name": "backlog_update_issue",
                "description": "Change an issue's fields as the user (including summary, description, type, status, assignee, priority, resolution, categories, versions, milestones, parentIssue, dates, hours, customFields, and an optional comment). Fields left out stay as they are. Only when the user asked for it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "key": key,
                        "summary": { "type": "string", "description": "New subject." },
                        "description": { "type": ["string", "null"], "description": "New description; null clears it." },
                        "issueType": { "type": ["string", "integer"], "description": "The issue type: its name or id." },
                        "status": { "type": ["string", "integer"], "description": "The status: its name or id." },
                        "assignee": assignee,
                        "priority": { "type": ["string", "integer"], "description": "The priority: its name or id." },
                        "resolution": name_or_id("The resolution"),
                        "categories": named_list("Categories"),
                        "versions": named_list("Affected versions"),
                        "milestones": named_list("Milestones"),
                        "parentIssue": { "type": ["string", "integer", "null"], "description": "Parent issue key, URL or numeric id; null clears it." },
                        "startDate": day("The start date"),
                        "dueDate": day("The due date"),
                        "estimatedHours": hours("Estimated hours"),
                        "actualHours": hours("Actual hours"),
                        "comment": { "type": "string", "description": "Comment to post along with the update." },
                        "notifiedUsers": user_list("Users to notify"),
                        "customFields": custom_fields_write,
                    },
                    "required": ["key"],
                },
            }),
        ),
        (
            Toolset::Issue,
            json!({
                "name": "backlog_delete_issue",
                "description": "Permanently delete a Backlog issue. Only when the user explicitly asked for it.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "key": key },
                    "required": ["key"],
                },
            }),
        ),
        (
            Toolset::Issue,
            json!({
                "name": "backlog_add_comment",
                "description": "Comment on an issue as the user (Backlog's Markdown or plain text). Only when the user asked for it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "key": key,
                        "content": { "type": "string" },
                        "notifiedUsers": user_list("Users to notify"),
                    },
                    "required": ["key", "content"],
                },
            }),
        ),
        (
            Toolset::Issue,
            json!({
                "name": "backlog_update_comment",
                "description": "Edit an existing comment on an issue (use the numeric #id shown by backlog_get_issue with comments: true). Only when the user asked for it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "key": key,
                        "commentId": { "type": "integer", "description": "The comment's numeric id." },
                        "content": { "type": "string", "description": "The replacement comment text." },
                    },
                    "required": ["key", "commentId", "content"],
                },
            }),
        ),
        (
            Toolset::Issue,
            json!({
                "name": "backlog_get_attachment",
                "description": "Download an issue's attachment to a local file (use an attachment id from backlog_get_issue). Raster images (PNG/JPEG/GIF/WebP up to 4 MB) are also returned inline as image content.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "key": key,
                        "attachmentId": { "type": "integer", "description": "The attachment's numeric id from backlog_get_issue." },
                    },
                    "required": ["key", "attachmentId"],
                },
            }),
        ),
        (
            Toolset::Issue,
            json!({
                "name": "backlog_manage_related_issue",
                "description": "Link or unlink a related issue. Only when the user asked for it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "key": key,
                        "target": { "type": ["string", "integer"], "description": "The related issue's key, URL or numeric id." },
                        "action": { "type": "string", "enum": ["add", "remove"] },
                    },
                    "required": ["key", "target", "action"],
                },
            }),
        ),
        // -------------------------------------------------------------------
        // Wiki
        // -------------------------------------------------------------------
        (
            Toolset::Wiki,
            json!({
                "name": "backlog_list_wikis",
                "description": "List wiki pages in a project (with total count, ids and tags), optionally filtered by keyword.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": project_key,
                        "keyword": { "type": "string", "description": "Search words in wiki page names or content." },
                    },
                    "required": ["project"],
                },
            }),
        ),
        (
            Toolset::Wiki,
            json!({
                "name": "backlog_get_wiki",
                "description": "Read a wiki page's full content, tags and attachments by its numeric id.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "wikiId": { "type": "integer", "description": "The wiki page's numeric id (from backlog_list_wikis)." },
                    },
                    "required": ["wikiId"],
                },
            }),
        ),
        (
            Toolset::Wiki,
            json!({
                "name": "backlog_create_wiki",
                "description": "Create a new wiki page in a project. Only when the user asked for it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": project_key,
                        "name": { "type": "string", "description": "Wiki page title." },
                        "content": { "type": "string", "description": "Wiki page body." },
                        "mailNotify": { "type": "boolean", "description": "Send email notification (default false)." },
                    },
                    "required": ["project", "name", "content"],
                },
            }),
        ),
        (
            Toolset::Wiki,
            json!({
                "name": "backlog_update_wiki",
                "description": "Update an existing wiki page's title or content. Only when the user asked for it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "wikiId": { "type": "integer", "description": "The wiki page's numeric id." },
                        "name": { "type": "string", "description": "New title." },
                        "content": { "type": "string", "description": "New body." },
                        "mailNotify": { "type": "boolean", "description": "Send email notification (default false)." },
                    },
                    "required": ["wikiId"],
                },
            }),
        ),
        // -------------------------------------------------------------------
        // Document
        // -------------------------------------------------------------------
        (
            Toolset::Document,
            json!({
                "name": "backlog_get_document_tree",
                "description": "Get the hierarchical document tree (folders and nested documents with ids) for a project.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": project_key,
                        "includeTrash": { "type": "boolean", "description": "Also show trashed documents." },
                    },
                    "required": ["project"],
                },
            }),
        ),
        (
            Toolset::Document,
            json!({
                "name": "backlog_list_documents",
                "description": "List or search documents in one or more projects.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": project_key,
                        "projects": { "type": "array", "items": { "type": ["string", "integer"] }, "description": "Multiple project keys or ids." },
                        "keyword": { "type": "string" },
                        "sort": { "type": "string", "enum": ["created", "updated"] },
                        "order": { "type": "string", "enum": ["asc", "desc"] },
                        "offset": { "type": "integer", "description": "Pagination offset (default 0)." },
                        "limit": { "type": "integer", "description": "Max documents to return (default 20, max 100)." },
                    },
                },
            }),
        ),
        (
            Toolset::Document,
            json!({
                "name": "backlog_get_document",
                "description": "Read a Backlog document's metadata and plain text content (paginated via start/limit; follow nextStart for long docs). comments: true also includes document comments and replies.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "documentId": { "type": "string", "description": "The document's UUID id (from backlog_get_document_tree or backlog_list_documents)." },
                        "start": { "type": "integer", "description": "Character offset to start reading from (default 0)." },
                        "limit": { "type": "integer", "description": "Max characters to return (default 32000)." },
                        "comments": { "type": "boolean", "description": "Also include comments and their replies." },
                    },
                    "required": ["documentId"],
                },
            }),
        ),
        (
            Toolset::Document,
            json!({
                "name": "backlog_create_document",
                "description": "Create a new document in a project. Only when the user asked for it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": project_key,
                        "title": { "type": "string", "description": "Document title." },
                        "content": { "type": "string", "description": "Document body (Markdown)." },
                        "emoji": { "type": "string", "description": "Optional emoji icon." },
                        "parentId": { "type": "string", "description": "Parent document UUID to nest under." },
                        "addLast": { "type": "boolean", "description": "Append at the end of siblings (default false)." },
                    },
                    "required": ["project"],
                },
            }),
        ),
        // -------------------------------------------------------------------
        // Git & Pull Requests
        // -------------------------------------------------------------------
        (
            Toolset::Git,
            json!({
                "name": "backlog_list_git_repositories",
                "description": "List Git repositories in a project (with HTTP/SSH URLs), or get details for one repository when repo is given.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": project_key,
                        "repo": repo,
                    },
                    "required": ["project"],
                },
            }),
        ),
        (
            Toolset::Git,
            json!({
                "name": "backlog_list_pull_requests",
                "description": "List or count pull requests in a Backlog Git repository.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": project_key,
                        "repo": repo,
                        "statuses": {
                            "type": "array",
                            "items": { "type": ["string", "integer"] },
                            "description": "Filter by status: \"open\" (1), \"closed\" (2), \"merged\" (3).",
                        },
                        "assignee": { "type": ["string", "integer", "array"], "description": "\"me\", member name/id, or array." },
                        "createdBy": { "type": ["string", "integer", "array"], "description": "Creator: \"me\", member name/id, or array." },
                        "issue": { "type": ["string", "integer"], "description": "Filter by linked issue key, URL or id." },
                        "offset": { "type": "integer" },
                        "limit": { "type": "integer", "description": "Max pull requests (default 20, max 100)." },
                        "countOnly": { "type": "boolean", "description": "Return only the total count." },
                    },
                    "required": ["project", "repo"],
                },
            }),
        ),
        (
            Toolset::Git,
            json!({
                "name": "backlog_get_pull_request",
                "description": "Read a pull request's details, branches, linked issue and description; comments: true adds its comments with #commentId.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": project_key,
                        "repo": repo,
                        "number": { "type": "integer", "description": "Pull request number." },
                        "comments": { "type": "boolean" },
                        "commentOrder": { "type": "string", "enum": ["asc", "desc"] },
                        "commentCount": { "type": "integer" },
                        "commentMinId": { "type": "integer" },
                        "commentMaxId": { "type": "integer" },
                    },
                    "required": ["project", "repo", "number"],
                },
            }),
        ),
        (
            Toolset::Git,
            json!({
                "name": "backlog_create_pull_request",
                "description": "Create a pull request in a Backlog Git repository. Only when the user asked for it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": project_key,
                        "repo": repo,
                        "summary": { "type": "string", "description": "Pull request title." },
                        "description": { "type": "string", "description": "Pull request description." },
                        "base": { "type": "string", "description": "Target base branch (e.g. main)." },
                        "branch": { "type": "string", "description": "Source feature branch." },
                        "issue": { "type": ["string", "integer"], "description": "Linked issue key, URL or numeric id." },
                        "assignee": assignee,
                        "notifiedUsers": user_list("Users to notify"),
                    },
                    "required": ["project", "repo", "summary", "description", "base", "branch"],
                },
            }),
        ),
        (
            Toolset::Git,
            json!({
                "name": "backlog_update_pull_request",
                "description": "Update a pull request's title, description, linked issue, assignee or add a comment. Backlog's API cannot merge, close or reopen a pull request. Only when the user asked for it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": project_key,
                        "repo": repo,
                        "number": { "type": "integer", "description": "Pull request number." },
                        "summary": { "type": "string" },
                        "description": { "type": "string" },
                        "issue": { "type": ["string", "integer", "null"], "description": "Linked issue key, URL or numeric id; null clears it." },
                        "assignee": assignee,
                        "notifiedUsers": user_list("Users to notify"),
                        "comment": { "type": "string", "description": "Comment to post with the update." },
                    },
                    "required": ["project", "repo", "number"],
                },
            }),
        ),
        (
            Toolset::Git,
            json!({
                "name": "backlog_comment_pull_request",
                "description": "Add a comment to a pull request, or edit an existing one when commentId is provided. Only when the user asked for it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": project_key,
                        "repo": repo,
                        "number": { "type": "integer", "description": "Pull request number." },
                        "content": { "type": "string" },
                        "commentId": { "type": "integer", "description": "Provide to edit an existing comment instead of adding a new one." },
                        "notifiedUsers": user_list("Users to notify (when adding a comment)"),
                    },
                    "required": ["project", "repo", "number", "content"],
                },
            }),
        ),
        // -------------------------------------------------------------------
        // Notifications & Watchings
        // -------------------------------------------------------------------
        (
            Toolset::Notifications,
            json!({
                "name": "backlog_list_notifications",
                "description": "List the connected user's Backlog notifications along with the unread count.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit": { "type": "integer", "description": "Max notifications (default 20, max 100)." },
                        "minId": { "type": "integer" },
                        "maxId": { "type": "integer" },
                        "order": { "type": "string", "enum": ["asc", "desc"] },
                    },
                },
            }),
        ),
        (
            Toolset::Notifications,
            json!({
                "name": "backlog_mark_notifications_read",
                "description": "Mark one notification as read (by id), or reset the unread notification count (all: true). Only when the user asked for it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer", "description": "Notification id to mark as read." },
                        "all": { "type": "boolean", "description": "Reset the unread notification count for all notifications." },
                    },
                },
            }),
        ),
        (
            Toolset::Issue,
            json!({
                "name": "backlog_list_watchings",
                "description": "List watched issues (and total count) for the connected user or a named user.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "user": { "type": ["string", "integer"], "description": "User: \"me\" (default), member name or id." },
                        "unreadOnly": { "type": "boolean", "description": "Only show watches with unread updates." },
                        "sort": { "type": "string", "enum": ["created", "updated", "issueUpdated"] },
                        "order": { "type": "string", "enum": ["asc", "desc"] },
                        "offset": { "type": "integer" },
                        "limit": { "type": "integer", "description": "Max items (default 20, max 100)." },
                    },
                },
            }),
        ),
        (
            Toolset::Issue,
            json!({
                "name": "backlog_manage_watching",
                "description": "Add, update note, delete or mark read a watched issue. Only when the user asked for it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["add", "update", "delete", "mark_read"] },
                        "issue": { "type": ["string", "integer"], "description": "Issue key, URL or id (required for add)." },
                        "watchId": { "type": "integer", "description": "Watching id from backlog_list_watchings (required for update/delete/mark_read)." },
                        "note": { "type": "string", "description": "Watch note (optional for add, required for update)." },
                    },
                    "required": ["action"],
                },
            }),
        ),
    ]
}
