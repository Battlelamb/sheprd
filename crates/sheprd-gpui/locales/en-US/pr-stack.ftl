# A stacked pull request's page: the chip that says which layer of its GitHub
# stack it is, and the popover it opens.

# $position counts layers from the bottom, 1 first; $size is how many there are.
pr-stack-chip = { $position }/{ $size }
pr-stack-chip-label = Layer { $position } of { $size } in a stack
pr-stack-popover = Stack
# $size is how many layers the stack has; $base is the branch it merges into.
pr-stack-summary = Stack of { $size } onto { $base } · bottom to top
# $count layers were read of the stack's $size.
pr-stack-truncated = Showing the first { $count } of { $size } layers.
# $number is a pull request's number.
pr-stack-open = Open #{ $number }

# The verdict for merging at this layer, which also merges the open layers below.
pr-stack-cant-merge = Can't merge yet
pr-stack-ready = { $count ->
    [one] Ready to merge
   *[other] Ready to merge { $count } pull requests
}
pr-stack-ready-detail = { $count ->
    [one] Nothing below it is still open.
   *[other] Merging it also merges the open pull requests below it, bottom first.
}
pr-stack-merged = Merged
pr-stack-merged-detail = This pull request and every one below it merged.
pr-stack-unread = Can't tell yet
pr-stack-unread-detail = Not every pull request below this one could be read, so what merging here would land is not known. Check the stack on GitHub.

# Why a layer cannot merge. $who is "#1362" or pr-stack-this.
pr-stack-this = This pull request
pr-stack-why-closed = { $who } was closed.
pr-stack-why-draft = { $who } is a draft.
pr-stack-why-conflicts = { $who } has merge conflicts.
pr-stack-why-checks-failed = { $who } has a failed check.
pr-stack-why-changes-requested = { $who } has changes requested.
pr-stack-why-checks-running = { $who } is waiting for its checks.
pr-stack-why-review-required = { $who } needs a review.
pr-stack-why-behind = { $who } is behind its base branch.
pr-stack-why-held = { $who } is held by branch protection.
pr-stack-why-undecided = { $who } is still being checked by GitHub.

# A layer's status under its number, a word or two.
pr-stack-status-merged = Merged
pr-stack-status-ready = Ready
pr-stack-status-closed = Closed
pr-stack-status-draft = Draft
pr-stack-status-conflicts = Conflicts
pr-stack-status-checks-failed = Checks failed
pr-stack-status-changes-requested = Changes requested
pr-stack-status-checks-running = Checks running
pr-stack-status-review-required = Review required
pr-stack-status-behind = Behind
pr-stack-status-held = Blocked
pr-stack-status-undecided = Checking

# The pull request's entry in its base branch's merge queue. $position counts from 1.
pr-stack-queue-chip = Queued { $position }
pr-stack-queued = In the merge queue, position { $position }
# $minutes is GitHub's estimate, rounded up.
pr-stack-queue-eta = About { $minutes } min to merge
pr-stack-queue-queued = Waiting for its turn.
pr-stack-queue-awaiting-checks = Running the checks of its merge group.
pr-stack-queue-mergeable = Its checks passed; it merges next.
pr-stack-queue-unmergeable = It can't merge and will leave the queue.
pr-stack-queue-locked = Locked while GitHub merges it.
pr-stack-queue-unknown = Queued.
