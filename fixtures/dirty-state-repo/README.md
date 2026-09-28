# Dirty-state reproduction fixture

The initial commit passes `sh check.sh`. The end-to-end test changes
`state.env` to `MODE=broken` without committing it, captures the failure, and
proves BugParcel reconstructs that dirty state in a detached worktree.
