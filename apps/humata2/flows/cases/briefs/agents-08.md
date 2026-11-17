# Brief agents-08: An agent forks onto another machine
Output: `apps/humata2/flows/cases/agents/08-forking-onto-another-machine.md`

Scenario: an agent on this Mac needs a GPU box. It asks to fork itself onto box-1; I approve; the fork runs on box-1 on a new branch with a copied conversation.

Steps: preview names branch and size and makes nothing; approval needed; code and context fork onto box-1; code only from box-1 to box-2 is a fresh agent and the parent keeps running.

Branches: box offline → refused, leaves nothing; a bad or taken typed branch refused up front; context-only onto another machine refused; context-only of a box session onto that box works; host restart mid-fork undone or finished.

Pointers: daemon/humatad/tests/fork_remote.rs (13), mcp.rs, workflow/milestones/FORK.md, MACH.md.
