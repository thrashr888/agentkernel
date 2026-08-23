# Durable tasks

`agentkernel task run` drains queued durable agent tasks using isolated Git
worktrees and sandboxes. Each worker claims tasks atomically, so multiple
agentkernel processes can safely share the queue.

```bash
# Use up to four task workers (the default)
agentkernel task run

# Bound active tasks explicitly
agentkernel task run --parallel 2
```

The command reports aggregate progress as tasks finish. A task's result is a
reviewable diff against its starting commit; failures remain recorded on the
task, and Ctrl-C cancels queued and running tasks after their cleanup path has
completed. `--parallel` accepts values from 1 through 64.
