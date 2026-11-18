# Traces: every agent session on this computer, with messages per day by
# agent, a filter and search, and one session's events. Agent, host, model,
# and folder names are shown as they are.

traces-title = Traces
traces-subtitle = Every agent session on this computer, indexed as it happens
traces-search = Search sessions, folders, branches, models
traces-starting = Reading agent sessions…
traces-failed = Traces could not read agent sessions: { $error }
traces-none = No sessions match. Widen the range or clear the search.
traces-more = { $count ->
        [one] { $count } more session; narrow the filter to see it
       *[other] { $count } more sessions; narrow the filter to see them
    }
traces-untitled = Untitled session
traces-close = Close

# The range and agent filters.
traces-range-week = 7 days
traces-range-month = 30 days
traces-range-quarter = 90 days
traces-range-year = 1 year
traces-agent-all = All agents
traces-agent-claude = Claude Code
traces-agent-codex = Codex
traces-agent-pi = Pi
traces-agent-omp = omp

# The totals over the sessions shown.
traces-tile-sessions = Sessions
traces-tile-hosts = { $count ->
        [one] on { $count } computer
       *[other] on { $count } computers
    }
traces-tile-messages = Messages
traces-tile-tools = Tool calls
traces-tile-errors = { $count ->
        [one] { $count } error
       *[other] { $count } errors
    }
traces-tile-tokens = Tokens

# The chart.
traces-chart-title = Messages per day
traces-chart-summary = { $messages } messages in { $days } days

# The table's columns.
traces-column-updated = Updated
traces-column-session = Session
traces-column-host = Computer
traces-column-model = Model
traces-column-messages = Msgs
traces-column-tools = Tools
traces-column-tokens = Tokens

# One session's facts and events.
traces-fact-agent = Agent
traces-fact-host = Computer
traces-fact-model = Model
traces-fact-folder = Folder
traces-fact-branch = Branch
traces-fact-started = Started
traces-fact-tokens = Tokens
traces-fact-tokens-value = { $input } in · { $output } out
traces-events-loading = Reading events…
traces-events-failed = Could not read this session's events: { $error }
traces-events-none = No events yet
traces-event-user = You
traces-event-agent = Agent
traces-event-thinking = Thinking
traces-event-tool = Tool
traces-event-result = Result
traces-event-error = Error
