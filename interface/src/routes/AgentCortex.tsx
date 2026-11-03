import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { AnimatePresence, motion } from "framer-motion";
import {
	api,
	type AutonomyFindingSummary,
	type CortexEvent,
	type CortexEventType,
} from "@/api/client";
import { CortexChatPanel } from "@/components/CortexChatPanel";
import { ResponsiveSplitPane } from "@/components/ResponsiveSplitPane";
import { useViewport } from "@/hooks/useViewport";
import { formatTimeAgo } from "@/lib/format";
import { IdeaIcon } from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { FilterButton, Button, Badge } from "@/ui";

const PAGE_SIZE = 50;

const EVENT_CATEGORY_COLORS: Record<string, string> = {
	bulletin_generated: "bg-blue-500/15 text-blue-400",
	bulletin_failed: "bg-red-500/15 text-red-400",
	maintenance_run: "bg-green-500/15 text-green-400",
	memory_merged: "bg-green-500/15 text-green-400",
	memory_decayed: "bg-green-500/15 text-green-400",
	memory_pruned: "bg-green-500/15 text-green-400",
	association_created: "bg-violet-500/15 text-violet-400",
	contradiction_flagged: "bg-violet-500/15 text-violet-400",
	worker_killed: "bg-amber-500/15 text-amber-400",
	branch_killed: "bg-amber-500/15 text-amber-400",
	circuit_breaker_tripped: "bg-amber-500/15 text-amber-400",
	observation_created: "bg-cyan-500/15 text-cyan-400",
	health_check: "bg-blue-500/15 text-blue-400",
	autonomy_cycle_started: "bg-cyan-500/15 text-cyan-400",
	autonomy_cycle_completed: "bg-emerald-500/15 text-emerald-400",
	autonomy_cycle_failed: "bg-red-500/15 text-red-400",
	autonomy_cycle_skipped_not_ready: "bg-amber-500/15 text-amber-400",
};

/** Groups for the filter pills — reduces clutter vs showing all 13 types. */
const FILTER_GROUPS: { label: string; types: CortexEventType[] }[] = [
	{ label: "Bulletin", types: ["bulletin_generated", "bulletin_failed"] },
	{ label: "Maintenance", types: ["maintenance_run", "memory_merged", "memory_decayed", "memory_pruned"] },
	{ label: "Health", types: ["worker_killed", "branch_killed", "circuit_breaker_tripped", "health_check"] },
	{ label: "Consolidation", types: ["association_created", "contradiction_flagged", "observation_created"] },
	{ label: "Autonomy", types: ["autonomy_cycle_started", "autonomy_cycle_completed", "autonomy_cycle_failed", "autonomy_cycle_skipped_not_ready"] },
];

function EventTypeBadge({ eventType }: { eventType: string }) {
	const color = EVENT_CATEGORY_COLORS[eventType] ?? "bg-app-darkBox text-ink-faint";
	const label = eventType.replace(/_/g, " ");
	return (
		<span className={`inline-flex items-center rounded px-1.5 py-0.5 text-tiny font-medium ${color}`}>
			{label}
		</span>
	);
}

function DetailsPanel({ details }: { details: Record<string, unknown> }) {
	return (
		<div className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-tiny">
			{Object.entries(details).map(([key, value]) => (
				<div key={key} className="contents">
					<span className="text-ink-faint">{key}</span>
					<span className="font-mono text-ink-dull">
						{typeof value === "object" ? JSON.stringify(value) : String(value)}
					</span>
				</div>
			))}
		</div>
	);
}

function SummaryStat({
	label,
	value,
	subtitle,
}: {
	label: string;
	value: string;
	subtitle: string;
}) {
	return (
		<div className="rounded-lg border border-app-line/50 bg-app-darkBox/30 p-3">
			<p className="text-tiny uppercase tracking-[0.16em] text-ink-faint">{label}</p>
			<p className="mt-2 font-plex text-xl text-ink">{value}</p>
			<p className="mt-1 text-xs text-ink-faint">{subtitle}</p>
		</div>
	);
}

function FindingRow({ finding }: { finding: AutonomyFindingSummary }) {
	return (
		<div className="rounded-lg border border-app-line/40 bg-app-darkBox/20 p-3">
			<div className="flex flex-wrap items-center gap-2">
				<Badge variant="blue">finding</Badge>
				<span className="text-tiny text-ink-faint">{finding.memory_type}</span>
				<span className="text-tiny text-ink-faint">
					{finding.importance.toFixed(2)} importance
				</span>
			</div>
			<p className="mt-2 line-clamp-3 text-sm text-ink-dull">{finding.content}</p>
			<p className="mt-2 text-tiny text-ink-faint">
				{formatTimeAgo(finding.created_at)}
				{finding.source ? ` • ${finding.source}` : ""}
			</p>
		</div>
	);
}

interface AgentCortexProps {
	agentId: string;
}

export function AgentCortex({ agentId }: AgentCortexProps) {
	const { isMobile, isTablet } = useViewport();
	const [typeFilter, setTypeFilter] = useState<CortexEventType | null>(null);
	const [groupFilter, setGroupFilter] = useState<string | null>(null);
	const [offset, setOffset] = useState(0);
	const [expandedId, setExpandedId] = useState<string | null>(null);
	const [chatOpen, setChatOpen] = useState(!(isMobile || isTablet));

	// Determine actual event_type filter from group or individual selection
	// For groups, we pass no event_type and filter client-side (API only supports single type)
	const activeGroupTypes = groupFilter
		? FILTER_GROUPS.find((g) => g.label === groupFilter)?.types ?? []
		: [];
	const isGroupFiltering = activeGroupTypes.length > 0;
	const serverLimit = isGroupFiltering ? 200 : PAGE_SIZE;
	const serverOffset = isGroupFiltering ? 0 : offset;

	const { data, isLoading, isError } = useQuery({
		queryKey: ["cortex-events", agentId, typeFilter, groupFilter, serverOffset, serverLimit],
		queryFn: () =>
			api.cortexEvents(agentId, {
				limit: serverLimit,
				offset: serverOffset,
				event_type: typeFilter ?? undefined,
			}),
		staleTime: 5_000,
	});
	const summaryQuery = useQuery({
		queryKey: ["cortex-summary", agentId],
		queryFn: () => api.cortexSummary(agentId, 6),
		staleTime: 5_000,
	});

	// Client-side filter for group selections
	// Group mode fetches latest 200 from server then filters client-side — pagination is
	// disabled and a banner is shown to make the "latest 200" snapshot behavior explicit.
	let events: CortexEvent[] = data?.events ?? [];
	let total = data?.total ?? 0;

	if (isGroupFiltering) {
		events = events.filter((event) => activeGroupTypes.includes(event.event_type));
		total = events.length;
	}

	const hasNext = isGroupFiltering ? false : offset + PAGE_SIZE < total;
	const hasPrev = isGroupFiltering ? false : offset > 0;

	const handleFilterChange = (group: string | null, type_: CortexEventType | null) => {
		setGroupFilter(group);
		setTypeFilter(type_);
		setOffset(0);
		setExpandedId(null);
	};

	const primary = (
		<div className="flex h-full flex-col overflow-hidden">
				<div className="border-b border-app-line/50 bg-app-darkBox/10 px-3 py-3 md:px-6">
					<div className="grid gap-3 xl:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]">
						<div className="grid gap-3 md:grid-cols-3">
							<SummaryStat
								label="Recent Events"
								value={String(summaryQuery.data?.recent_events.length ?? 0)}
								subtitle={
									summaryQuery.data?.recent_events[0]
										? `Latest ${formatTimeAgo(summaryQuery.data.recent_events[0].created_at)}`
										: "No recent cortex events"
								}
							/>
							<SummaryStat
								label="Autonomy Cycles"
								value={String(summaryQuery.data?.autonomy_cycles.length ?? 0)}
								subtitle={
									summaryQuery.data?.autonomy_cycles[0]
										? summaryQuery.data.autonomy_cycles[0].event_type.replace(/_/g, " ")
										: "No recent autonomy activity"
								}
							/>
							<SummaryStat
								label="Findings"
								value={String(summaryQuery.data?.recent_findings.length ?? 0)}
								subtitle={
									summaryQuery.data?.recent_findings[0]
										? `Latest ${formatTimeAgo(summaryQuery.data.recent_findings[0].created_at)}`
										: "No autonomy findings saved"
								}
							/>
						</div>
						<div className="grid gap-3 md:grid-cols-2">
							<div className="rounded-lg border border-app-line/50 bg-app-darkBox/30 p-3">
								<div className="mb-3 flex items-center justify-between gap-2">
									<h3 className="text-sm font-medium text-ink">Autonomy</h3>
									<span className="text-tiny text-ink-faint">Latest cycles</span>
								</div>
								{summaryQuery.isLoading ? (
									<p className="text-sm text-ink-faint">Loading autonomy summary...</p>
								) : summaryQuery.isError ? (
									<p className="text-sm text-red-400">Failed to load autonomy summary.</p>
								) : summaryQuery.data?.autonomy_cycles.length ? (
									<div className="space-y-2">
										{summaryQuery.data.autonomy_cycles.map((event) => (
											<div key={event.id} className="rounded-md border border-app-line/40 bg-app-darkBox/20 p-3">
												<div className="flex flex-wrap items-center gap-2">
													<EventTypeBadge eventType={event.event_type} />
													<span className="text-tiny text-ink-faint">
														{formatTimeAgo(event.created_at)}
													</span>
												</div>
												<p className="mt-2 text-sm text-ink-dull">{event.summary}</p>
											</div>
										))}
									</div>
								) : (
									<p className="text-sm text-ink-faint">No autonomy cycle events yet.</p>
								)}
							</div>
							<div className="rounded-lg border border-app-line/50 bg-app-darkBox/30 p-3">
								<div className="mb-3 flex items-center justify-between gap-2">
									<h3 className="text-sm font-medium text-ink">Findings</h3>
									<span className="text-tiny text-ink-faint">Recent saved observations</span>
								</div>
								{summaryQuery.isLoading ? (
									<p className="text-sm text-ink-faint">Loading autonomy findings...</p>
								) : summaryQuery.isError ? (
									<p className="text-sm text-red-400">Failed to load autonomy findings.</p>
								) : summaryQuery.data?.recent_findings.length ? (
									<div className="space-y-2">
										{summaryQuery.data.recent_findings.map((finding) => (
											<FindingRow key={finding.id} finding={finding} />
										))}
									</div>
								) : (
									<p className="text-sm text-ink-faint">No autonomy findings recorded yet.</p>
								)}
							</div>
						</div>
					</div>
				</div>
				{/* Filter bar */}
				<div className="flex flex-wrap items-center gap-1.5 border-b border-app-line/50 bg-app-darkBox/20 px-3 py-2 md:px-6">
					<button
						onClick={() => handleFilterChange(null, null)}
						className={`rounded-md px-2 py-1 text-tiny font-medium transition-colors ${
							!typeFilter && !groupFilter
								? "bg-accent/15 text-accent"
								: "text-ink-faint hover:text-ink-dull"
						}`}
					>
						All
					</button>
				{FILTER_GROUPS.map((group) => (
					<FilterButton
						key={group.label}
						onClick={() =>
							handleFilterChange(
								groupFilter === group.label ? null : group.label,
								null,
							)
						}
						active={groupFilter === group.label}
						colorClass={EVENT_CATEGORY_COLORS[group.types[0]]}
					>
						{group.label}
					</FilterButton>
				))}

					{/* Count + pagination + chat toggle */}
					<div className="flex w-full items-center justify-between gap-2 sm:ml-auto sm:w-auto sm:justify-end sm:gap-3">
						{isGroupFiltering ? (
							<span className="text-tiny text-ink-faint">
								Latest 200 events ({total} matching)
							</span>
						) : total > 0 && (
							<span className="text-tiny text-ink-faint">
								{offset + 1}-{Math.min(offset + PAGE_SIZE, total)} of {total}
							</span>
						)}
						{(hasPrev || hasNext) && (
							<div className="flex items-center gap-1">
								<button
									onClick={() => setOffset(Math.max(0, offset - PAGE_SIZE))}
									disabled={!hasPrev}
									className="rounded px-1.5 py-0.5 text-tiny text-ink-faint transition-colors hover:text-ink-dull disabled:opacity-30"
								>
									Prev
								</button>
								<button
									onClick={() => setOffset(offset + PAGE_SIZE)}
									disabled={!hasNext}
									className="rounded px-1.5 py-0.5 text-tiny text-ink-faint transition-colors hover:text-ink-dull disabled:opacity-30"
								>
									Next
								</button>
							</div>
						)}
					<div className="flex overflow-hidden rounded-md border border-app-line bg-app-darkBox">
						<Button
							onClick={() => setChatOpen(!chatOpen)}
							variant={chatOpen ? "secondary" : "ghost"}
							size="icon"
							className={chatOpen ? "bg-app-selected text-ink" : ""}
							title="Toggle cortex chat"
						aria-label="Toggle cortex chat"
						>
							<HugeiconsIcon icon={IdeaIcon} className="h-4 w-4" />
						</Button>
					</div>
					</div>
				</div>

				{/* Event list */}
				{isLoading ? (
					<div className="flex flex-1 items-center justify-center">
						<div className="flex items-center gap-2 text-ink-dull">
							<div className="h-2 w-2 animate-pulse rounded-full bg-accent" />
							Loading cortex events...
						</div>
					</div>
				) : isError ? (
					<div className="flex flex-1 items-center justify-center">
						<p className="text-sm text-red-400">Failed to load cortex events</p>
					</div>
				) : events.length === 0 ? (
					<div className="flex flex-1 items-center justify-center">
						<p className="text-sm text-ink-faint">
							{typeFilter || groupFilter ? "No events match this filter" : "No cortex events yet"}
						</p>
					</div>
				) : (
					<div className="flex-1 overflow-y-auto">
						<div className="flex flex-col">
							{events.map((event) => {
								const isExpanded = expandedId === event.id;
								return (
									<div key={event.id} className="border-b border-app-line/30">
										<button
											onClick={() => setExpandedId(isExpanded ? null : event.id)}
											className="flex w-full items-center gap-4 px-6 py-3 text-left transition-colors hover:bg-app-darkBox/30"
										>
											<span className="w-20 flex-shrink-0 text-tiny text-ink-faint">
												{formatTimeAgo(event.created_at)}
											</span>
											<EventTypeBadge eventType={event.event_type} />
											<span className="min-w-0 flex-1 truncate text-sm text-ink-dull">
												{event.summary}
											</span>
											{event.details && (
												<span className="flex-shrink-0 text-tiny text-ink-faint">
													{isExpanded ? "v" : ">"}
												</span>
											)}
										</button>

										<AnimatePresence>
											{isExpanded && event.details && (
												<motion.div
													initial={{ height: 0, opacity: 0 }}
													animate={{ height: "auto", opacity: 1 }}
													exit={{ height: 0, opacity: 0 }}
													transition={{ type: "spring", stiffness: 500, damping: 35 }}
													className="overflow-hidden"
												>
													<div className="border-t border-app-line/20 bg-app-darkBox/20 px-6 py-4">
														<DetailsPanel details={event.details} />
													</div>
												</motion.div>
											)}
										</AnimatePresence>
									</div>
								);
							})}
						</div>
					</div>
				)}
		</div>
	);

	const secondary = (
		<div className="h-full">
			<CortexChatPanel agentId={agentId} onClose={() => setChatOpen(false)} />
		</div>
	);

	return (
		<ResponsiveSplitPane
			primary={primary}
			secondary={secondary}
			showSecondary={chatOpen}
			onCloseSecondary={() => setChatOpen(false)}
			secondaryTitle="Cortex Chat"
			secondaryWidthClassName="w-[min(400px,40%)]"
		/>
	);
}
