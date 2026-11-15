@AGENTS.md

## Skills

- `/dag-walk [init|plan|resume|gate|close <ID>]` runs long, multi-milestone work as a DAG of agent lanes (`tools/skills/dag-walk/`). Each project keeps its state in a `workflow/` folder; Humata 2's is `apps/humata2/workflow/`. To enable the slash command in a checkout: `mkdir -p .claude/skills && ln -sfn ../../tools/skills/dag-walk .claude/skills/dag-walk`.
