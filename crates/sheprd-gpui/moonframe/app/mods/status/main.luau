-- The status bar: the connection, then the machine's load and quick commands.

local gauge = use "sheprd-gauge"

local function run(command: string)
	return { command = command }
end

-- A usage segment's height, and so the one line of them the bar shows.
local SEGMENT_HEIGHT = 20

-- Each usage provider's logo among Moonframe's bundled ones, which cannot
-- draw Sheprd's own (`assets/icons/providers`); a robot for the rest.
local LOGOS = {
	amp = "agent-amp",
	claude = "agent-claude",
	codex = "agent-codex",
	copilot = "agent-copilot",
	cursor = "agent-cursor",
	factory = "agent-droid",
	gemini = "agent-gemini",
	grok = "agent-grok",
	kiro = "agent-kiro",
	mistral = "brand-mistral",
	ollama = "brand-ollama",
	opencode = "agent-opencode",
	qwencloud = "agent-qwen",
	xai = "agent-grok",
}

-- One provider's usage, as Sheprd's own bar draws it: its logo, a meter of
-- the window closest to its limit, and the tightest windows' shares, each
-- in its severity's color (`app.sheprd.usage`, src/usage/hosted.rs). A
-- click opens its panel.
local function usage_segment(segment: any, theme: any)
	local row: any = {
		class = "usage-segment",
		align = "center",
		height = SEGMENT_HEIGHT,
		shrink = false,
		tooltip = segment.name,
		msg = { usage = segment.id },
		to = "provider:sheprd",
		style = { gap = 6, padding = { x = 6 }, radius = 4 },
		ui.icon { name = LOGOS[segment.id] or "bot", style = { color = theme.foreground } },
	}
	if segment.locked then
		table.insert(row, ui.icon { name = "lock", size = "small", style = { color = theme.muted } })
	end
	if segment.meter then
		table.insert(row, ui.row {
			width = 40,
			height = 5,
			shrink = false,
			style = { background = segment.meter.track, radius = 3 },
			ui.row { width = 40 * segment.meter.used, height = 5, style = { background = segment.meter.color, radius = 3 } },
		})
	end
	for index, label in segment.labels or {} do
		if index > 1 then
			table.insert(row, ui.text { "·", style = { color = theme.muted } })
		end
		table.insert(row, ui.text { label.text, style = { color = label.color } })
	end
	if segment.balance then
		table.insert(row, ui.text { segment.balance })
	end
	-- The numbers are the last good ones; the panel says why.
	if segment.stale then
		table.insert(row, ui.text { "!", style = { color = segment.stale_color } })
	end
	return ui.row(row)
end

-- The usage segments, then the button that reads them again; a host that
-- could not be read says so while there is nothing older. The segments
-- take what width the bar has left: they wrap onto a second line it clips,
-- so one that does not fit hides whole, as in Sheprd's own bar, and the
-- refresh button and the items after it stay in the window.
local function usage(state: any, theme: any)
	local segments: any = {
		class = "sheprd-usage-segments",
		align = "center",
		wrap = true,
		height = SEGMENT_HEIGHT,
		style = { gap = 4, clip = true },
	}
	for _, segment in state.segments do
		table.insert(segments, usage_segment(segment, theme))
	end
	local row: any = { class = "sheprd-usage", align = "center", style = { gap = 4 }, ui.row(segments) }
	if state.error then
		table.insert(row, ui.text { state.unavailable, tooltip = state.error, style = { color = theme.muted } })
	end
	table.insert(row, ui.row {
		class = "usage-refresh",
		align = "center",
		justify = "center",
		width = 18,
		height = 18,
		shrink = false,
		tooltip = state.refresh_label,
		msg = { usage_refresh = true },
		to = "provider:sheprd",
		style = { radius = 4 },
		ui.icon { name = "refresh-cw", size = "small", spin = state.busy == true, style = { color = theme.muted } },
	})
	return ui.row(row)
end

-- The selected host's load as Sheprd's own bar draws it
-- (`app.sheprd_load`, src/system_load/hosted.rs): CPU as a sparkline of its
-- recent samples, memory as a meter, each with its share, in their
-- severity's colors; the graphs only while the bar is detailed. The GPU,
-- when shown, follows as its own pair: busy share and video memory.
local function load_item(load: any)
	if load.unavailable then
		return ui.text { load.unavailable, style = { color = load.label_color } }
	end
	local row: any = { class = "sheprd-load", align = "center", shrink = false, style = { gap = 10 } }
	if load.cpu_label then
		table.insert(row, gauge {
			class = "sheprd-load-cpu",
			name = load.cpu_label,
			value = load.cpu,
			color = load.cpu_color,
			label_color = load.label_color,
			history = load.history,
			track = load.track,
		})
		if load.memory then
			table.insert(row, gauge {
				class = "sheprd-load-memory",
				name = load.memory_label,
				value = load.memory,
				color = load.memory_color,
				label_color = load.label_color,
				meter = load.meter,
				track = load.track,
			})
		end
	end
	local gpu = load.gpu
	if gpu then
		if gpu.busy then
			table.insert(row, gauge {
				class = "sheprd-load-gpu",
				name = gpu.busy_label,
				value = gpu.busy,
				color = gpu.busy_color,
				label_color = gpu.label_color,
				history = gpu.history,
				track = gpu.track,
			})
		end
		if gpu.memory then
			table.insert(row, gauge {
				class = "sheprd-load-vram",
				name = gpu.memory_label,
				value = gpu.memory,
				color = gpu.memory_color,
				label_color = gpu.label_color,
				meter = gpu.meter,
				track = gpu.track,
			})
		end
	end
	return ui.row(row)
end

local function item(icon: string, text: string, color: string, msg: any, to: string?, shown: string?)
	-- `[status_bar]` hides a button, or draws its icon alone, named by its
	-- tooltip, as Sheprd's own bar does.
	if shown == "hidden" then
		return false
	end
	local icon_only = shown == "icon"
	return ui.row {
		class = "status-item",
		align = "center",
		shrink = false,
		msg = msg,
		to = to,
		tooltip = if icon_only then text else nil,
		style = { gap = 5 },
		-- As Sheprd's own bar: text at the UI size, icons a size up.
		ui.icon { name = icon, size = "large", style = { color = color } },
		if icon_only then false else ui.text { text, style = { color = color } },
	}
end

return mod {
	styles = {
		[".usage-segment:hover"] = { background = "secondary" },
		[".usage-refresh:hover"] = { background = "secondary" },
	},

	init = function(previous)
		return previous or {}
	end,

	update = function() end,

	view = function(_, app)
		local sheprd = app.sheprd or { agents = {} }
		local working = 0
		for _, agent in sheprd.agents do
			if agent.status == "working" then
				working += 1
			end
		end
		local theme = app.sheprd_theme or {}
		local buttons = sheprd.status_bar or {}
		return ui.row {
			class = "sheprd-status",
			grow = true,
			align = "center",
			style = { background = theme.surface, gap = 14, padding = { x = 12 } },
			if sheprd.status then item("circle-alert", sheprd.status, theme.danger) else false,
			if working > 0 then item("bot", `{working} working`, theme.accent) else false,
			if sheprd.usage then usage(sheprd.usage, theme) else false,
			ui.spacer(),
			if app.sheprd_load then load_item(app.sheprd_load) else false,
			item("palette", "Theme", theme.subtext, run "themes", "provider:sheprd", buttons.theme),
			item("command", "Commands", theme.subtext, run "command_palette", "provider:sheprd", buttons.commands),
		}
	end,
}
