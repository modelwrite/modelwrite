# The modelwrite agent

Slice 1 provides the contract. Slice 5 provides the agent. This file is the seam
between them, so the agent is designed for rather than retrofitted.

## What exists after Slice 1

- The MCP server (mw-mcp): okf.validate, okf.diff, graph.stats, gate.run, plus the
  resources mw://okf/1.0/spec and mw://evidence/latest.
- The published manifest docs/agents/mcp-tools.json: tool names, input schemas and the
  stability rule.
- The rule pack agents/CLAUDE.md: the invariants any agent, ours or third party, must
  respect.

## What Slice 5 adds

- agent/: the shipped MCP client. Skill packs, a provider abstraction (local model
  first, customer endpoint or approved cloud optionally), step and token budgets, and a
  replayable run log.
- The model-edit API and its tools: model.read, model.propose, rules.check,
  patterns.list, patterns.instantiate, evidence.write.
- Generation, repair and review skills, and the agent evaluation harness that measures
  them.

## The loop

intake -> propose instructions on the model-edit API -> apply them to a draft branch ->
run the rules -> run the gate -> repair what the gate rejects -> draft commit for human
review -> evidence.

Patterns are the generation substrate: the agent instantiates and parameterises known
patterns rather than inventing structure, which is what makes generated models sound
before anyone reviews them.

## Rules that bind the agent

1. The gate is the only authority on correctness; no agent, log line or tool parameter
   can mark a run as passed.
2. Every action is an instruction on the model-edit API, never a direct edit.
3. Every iteration ends in a gate run, and the output is a draft commit for human review.
4. Ambiguity is a question, not an invention: the agent asks rather than guessing.
5. Every run is replayable: prompts, tool calls and results are logged, and step, token
   and wall-clock budgets are enforced.
