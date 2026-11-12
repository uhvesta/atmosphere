//! `humata mcp`'s admin tools, in dev builds only (`--config=humata-admin`):
//! an agent drives the board as the person would, each action waiting for
//! the person's approval in Humata unless a grant they gave covers it.

use humata_wire::pb;
use serde_json::json;

use crate::approved::Client;
use crate::control::{Arguments, Effect, Tool, ask, lines, string, unexpected};
use crate::print;

/// An admin tool's description, with what an agent is told of each.
macro_rules! waits {
    ($description:literal) => {
        concat!(
            $description,
            " It waits for the person's approval in Humata, unless a grant covers it; if they don't decide within 20 seconds it answers with an approval id to check with approval_status."
        )
    };
}

pub fn tools() -> Vec<Tool> {
    let proposal = || string("The proposal's id, from list_proposals");
    let session =
        || string("The session that proposed it; leave out for an outside agent's proposal");
    let item = || string("The task's id, from board");
    vec![
        Tool {
            name: "list_proposals",
            description: "The tasks and spec edits agents outside Humata proposed for a project's board, oldest first.",
            properties: json!({"project_id": string("The project's id, from list_projects")}),
            required: &["project_id"],
            effect: Effect::Reads,
        },
        Tool {
            name: "apply_proposal",
            description: waits!(
                "Applies a proposal as the person would: adds the task or changes the spec."
            ),
            properties: json!({"proposal_id": proposal(), "session_id": session()}),
            required: &["proposal_id"],
            effect: Effect::Changes,
        },
        Tool {
            name: "decline_proposal",
            description: waits!("Declines a proposal as the person would."),
            properties: json!({"proposal_id": proposal(), "session_id": session()}),
            required: &["proposal_id"],
            effect: Effect::Changes,
        },
        Tool {
            name: "create_item",
            description: waits!("Adds a task to a project's board as the person would."),
            properties: json!({
                "project_id": string("The project's id, from list_projects"),
                "title": string("The task's title"),
                "spec": string("Its spec, in Markdown"),
                "parent_id": string("The task to put it under, if any"),
            }),
            required: &["project_id", "title"],
            effect: Effect::Changes,
        },
        Tool {
            name: "update_item",
            description: waits!("Changes a task's title, spec or status as the person would."),
            properties: json!({
                "item_id": item(),
                "title": string("Its new title"),
                "spec": string("Its new spec, in Markdown, whole"),
                "status": {"type": "string", "enum": ["todo", "in_progress", "in_review", "done", "wont_do"]},
            }),
            required: &["item_id"],
            effect: Effect::Changes,
        },
        Tool {
            name: "delete_item",
            description: waits!(
                "Deletes a task and those under it. Destructive; only a grant naming destructive actions covers it."
            ),
            properties: json!({"item_id": item()}),
            required: &["item_id"],
            effect: Effect::Destroys,
        },
        Tool {
            name: "revert_change",
            description: waits!(
                "Reverts one change in a board's history. Destructive; only a grant naming destructive actions covers it."
            ),
            properties: json!({"change_id": {"type": "integer", "description": "The change's id"}}),
            required: &["change_id"],
            effect: Effect::Destroys,
        },
        Tool {
            name: "approval_status",
            description: "How an admin action waiting for the person's approval stands, and what it answered once approved.",
            properties: json!({"approval_id": string("The approval's id")}),
            required: &["approval_id"],
            effect: Effect::Reads,
        },
    ]
}

fn status(given: &str) -> Result<pb::ItemStatus, String> {
    Ok(match given {
        "todo" => pb::ItemStatus::Todo,
        "in_progress" => pb::ItemStatus::InProgress,
        "in_review" => pb::ItemStatus::InReview,
        "done" => pb::ItemStatus::Done,
        "wont_do" => pb::ItemStatus::WontDo,
        other => {
            return Err(format!(
                "there's no status {other:?}; give todo, in_progress, in_review, done or wont_do"
            ));
        }
    })
}

fn optional(arguments: &Arguments<'_>, field: &str) -> Option<String> {
    arguments.0[field].as_str().map(str::to_owned)
}

/// What admin tool `name` answers; None if it isn't one.
pub async fn call(
    client: &Client,
    name: &str,
    arguments: &Arguments<'_>,
) -> Option<Result<String, String>> {
    Some(match name {
        "list_proposals" => list_proposals(client, arguments).await,
        "apply_proposal" => {
            let request = pb::request::Kind::ApplyProposal(pb::ApplyProposal {
                session_id: arguments.text("session_id"),
                proposal_id: match arguments.required("proposal_id") {
                    Ok(id) => id,
                    Err(error) => return Some(Err(error)),
                },
                title: None,
                spec: None,
            });
            match ask(client, request).await {
                Ok(pb::response::Kind::ProposalApplied(applied)) => Ok(format!(
                    "Applied; the task is {}.",
                    applied.item.map(|item| item.id).unwrap_or_default()
                )),
                Ok(other) => Err(unexpected(&other)),
                Err(error) => Err(error),
            }
        }
        "decline_proposal" => {
            let request = pb::request::Kind::DeclineProposal(pb::DeclineProposal {
                session_id: arguments.text("session_id"),
                proposal_id: match arguments.required("proposal_id") {
                    Ok(id) => id,
                    Err(error) => return Some(Err(error)),
                },
            });
            match ask(client, request).await {
                Ok(pb::response::Kind::ProposalDeclined(_)) => Ok("Declined.".to_owned()),
                Ok(other) => Err(unexpected(&other)),
                Err(error) => Err(error),
            }
        }
        "create_item" => create_item(client, arguments).await,
        "update_item" => update_item(client, arguments).await,
        "delete_item" => {
            let request = pb::request::Kind::DeleteItem(pb::DeleteItem {
                item_id: match arguments.required("item_id") {
                    Ok(id) => id,
                    Err(error) => return Some(Err(error)),
                },
                surface: pb::ChangeSurface::Mcp.into(),
            });
            match ask(client, request).await {
                Ok(pb::response::Kind::ItemDeleted(_)) => Ok("Deleted.".to_owned()),
                Ok(other) => Err(unexpected(&other)),
                Err(error) => Err(error),
            }
        }
        "revert_change" => {
            let Some(change_id) = arguments.number("change_id") else {
                return Some(Err("give change_id".to_owned()));
            };
            let request = pb::request::Kind::RevertBoardChange(pb::RevertBoardChange {
                change_id,
                surface: pb::ChangeSurface::Mcp.into(),
            });
            match ask(client, request).await {
                Ok(pb::response::Kind::BoardChangeReverted(_)) => Ok("Reverted.".to_owned()),
                Ok(other) => Err(unexpected(&other)),
                Err(error) => Err(error),
            }
        }
        "approval_status" => {
            let id = match arguments.required("approval_id") {
                Ok(id) => id,
                Err(error) => return Some(Err(error)),
            };
            match client
                .wait_for(&id, crate::approved::Wait::For(std::time::Duration::ZERO))
                .await
            {
                Ok(approval) => {
                    let line = print::approval(&approval);
                    match crate::approved::outcome(*approval) {
                        Ok(_) => Ok(format!("{line}\nIt ran.")),
                        Err(error) => Ok(format!("{line}\n{}", humata_wire::describe(&error))),
                    }
                }
                Err(error) => Err(humata_wire::describe(&error)),
            }
        }
        _ => return None,
    })
}

async fn list_proposals(client: &Client, arguments: &Arguments<'_>) -> Result<String, String> {
    let request = pb::request::Kind::ListProposals(pb::ListProposals {
        session_id: String::new(),
        project_id: arguments.required("project_id")?,
    });
    match ask(client, request).await? {
        pb::response::Kind::Proposals(proposals) => Ok(lines(
            &proposals.proposals,
            print::proposal,
            "No agent outside Humata proposed anything for this board.",
        )),
        other => Err(unexpected(&other)),
    }
}

async fn create_item(client: &Client, arguments: &Arguments<'_>) -> Result<String, String> {
    let request = pb::request::Kind::CreateItem(pb::CreateItem {
        project_id: arguments.required("project_id")?,
        title: arguments.required("title")?,
        spec: optional(arguments, "spec").unwrap_or_default(),
        parent_id: arguments.text("parent_id"),
        surface: pb::ChangeSurface::Mcp.into(),
        ..Default::default()
    });
    match ask(client, request).await? {
        pb::response::Kind::ItemCreated(pb::ItemCreated { item: Some(item) }) => {
            Ok(format!("Added task {}.", item.id))
        }
        other => Err(unexpected(&other)),
    }
}

async fn update_item(client: &Client, arguments: &Arguments<'_>) -> Result<String, String> {
    let status = optional(arguments, "status")
        .map(|given| status(&given))
        .transpose()?;
    let request = pb::request::Kind::UpdateItem(pb::UpdateItem {
        item_id: arguments.required("item_id")?,
        title: optional(arguments, "title"),
        spec: optional(arguments, "spec"),
        status: status.map(Into::into),
        surface: pb::ChangeSurface::Mcp.into(),
        ..Default::default()
    });
    match ask(client, request).await? {
        pb::response::Kind::ItemUpdated(_) => Ok("Changed.".to_owned()),
        other => Err(unexpected(&other)),
    }
}
