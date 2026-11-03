import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api, type CapabilityRecord, type CapabilityStatus } from "@/api/client";
import { Badge } from "@/ui";
import { formatTimeAgo } from "@/lib/format";

interface AgentToolsProps {
	agentId: string;
}

const STATUS_FILTERS: Array<CapabilityStatus | "all"> = [
	"all",
	"degraded",
	"unavailable",
	"available",
];

function statusVariant(status: CapabilityStatus): "green" | "amber" | "red" {
	switch (status) {
		case "available":
			return "green";
		case "degraded":
			return "amber";
		case "unavailable":
			return "red";
	}
}

function confidenceLabel(confidence: CapabilityRecord["confidence"]): string {
	return confidence.replace(/_/g, " ");
}

function formatBytes(bytes: number): string {
	if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
	if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
	return `${bytes} B`;
}

export function AgentTools({ agentId }: AgentToolsProps) {
	const [statusFilter, setStatusFilter] = useState<CapabilityStatus | "all">("all");
	const [search, setSearch] = useState("");

	const toolsQuery = useQuery({
		queryKey: ["agent-tools", agentId],
		queryFn: () => api.agentTools(agentId),
		refetchInterval: 15_000,
	});
	const binariesQuery = useQuery({
		queryKey: ["tool-binaries"],
		queryFn: () => api.toolBinaries(),
		refetchInterval: 30_000,
	});

	const capabilities = toolsQuery.data?.snapshot.capabilities ?? [];
	const filteredCapabilities = useMemo(() => {
		const term = search.trim().toLowerCase();
		return [...capabilities]
			.filter((capability) =>
				statusFilter === "all" ? true : capability.status === statusFilter,
			)
			.filter((capability) => {
				if (!term) return true;
				return [
					capability.name,
					capability.subject,
					capability.invocation_path,
					capability.reason_code ?? "",
					capability.detail ?? "",
				]
					.join(" ")
					.toLowerCase()
					.includes(term);
			})
			.sort((left, right) => {
				const priority = { unavailable: 0, degraded: 1, available: 2 };
				const statusOrder = priority[left.status] - priority[right.status];
				if (statusOrder !== 0) return statusOrder;
				return left.name.localeCompare(right.name);
			});
	}, [capabilities, search, statusFilter]);

	return (
		<div className="h-full overflow-y-auto">
			<div className="mx-auto flex w-full max-w-7xl flex-col gap-6 px-4 py-5 md:px-6">
				<div className="grid gap-3 md:grid-cols-4">
					<SummaryCard
						label="Available"
						value={toolsQuery.data?.snapshot.summary.available ?? 0}
						tone="green"
					/>
					<SummaryCard
						label="Degraded"
						value={toolsQuery.data?.snapshot.summary.degraded ?? 0}
						tone="amber"
					/>
					<SummaryCard
						label="Unavailable"
						value={toolsQuery.data?.snapshot.summary.unavailable ?? 0}
						tone="red"
					/>
					<SummaryCard
						label="Binaries"
						value={binariesQuery.data?.binaries.length ?? 0}
						tone="default"
						meta={
							toolsQuery.data?.snapshot.generated_at
								? `Updated ${formatTimeAgo(toolsQuery.data.snapshot.generated_at)}`
								: "Waiting for capability snapshot"
						}
					/>
				</div>

				<section className="rounded-xl border border-app-line bg-app-box">
					<div className="border-b border-app-line px-4 py-3">
						<h2 className="font-plex text-sm font-medium text-ink">Doctor</h2>
						<p className="mt-1 text-xs text-ink-faint">
							Grouped operator diagnoses derived from degraded and unavailable capability records.
						</p>
					</div>
					<div className="grid gap-3 p-4 md:grid-cols-2">
						{toolsQuery.isLoading ? (
							<p className="text-sm text-ink-faint">Loading doctor report...</p>
						) : toolsQuery.isError ? (
							<p className="text-sm text-red-400">Failed to load doctor report.</p>
						) : toolsQuery.data?.doctor.diagnoses.length ? (
							toolsQuery.data.doctor.diagnoses.map((diagnosis) => (
								<div
									key={diagnosis.class}
									className="rounded-lg border border-app-line/70 bg-app-darkBox/40 p-4"
								>
									<div className="flex items-center justify-between gap-3">
										<h3 className="text-sm font-medium text-ink">{diagnosis.class.replace(/_/g, " ")}</h3>
										<Badge variant="amber">{diagnosis.capability_ids.length} hits</Badge>
									</div>
									<p className="mt-2 text-sm text-ink-dull">{diagnosis.summary}</p>
									<p className="mt-2 text-xs text-ink-faint">{diagnosis.remediation}</p>
								</div>
							))
						) : (
							<p className="text-sm text-emerald-400">No active tool-health diagnoses.</p>
						)}
					</div>
				</section>

				<section className="rounded-xl border border-app-line bg-app-box">
					<div className="border-b border-app-line px-4 py-3">
						<h2 className="font-plex text-sm font-medium text-ink">Durable Binaries</h2>
						<p className="mt-1 break-all font-mono text-xs text-ink-faint">
							{binariesQuery.data?.tools_bin ?? "Loading tools/bin path..."}
						</p>
					</div>
					<div className="p-4">
						{binariesQuery.isLoading ? (
							<p className="text-sm text-ink-faint">Loading tool binaries...</p>
						) : binariesQuery.isError ? (
							<p className="text-sm text-red-400">Failed to load tool binaries.</p>
						) : binariesQuery.data?.binaries.length ? (
							<div className="grid gap-3 md:grid-cols-2">
								{binariesQuery.data.binaries.map((binary) => (
									<div
										key={binary.name}
										className="rounded-lg border border-app-line/70 bg-app-darkBox/40 p-4"
									>
										<div className="flex items-center justify-between gap-3">
											<h3 className="font-mono text-sm text-ink">{binary.name}</h3>
											<Badge variant="outline">{formatBytes(binary.size)}</Badge>
										</div>
										<p className="mt-2 text-xs text-ink-faint">
											{binary.modified
												? `Modified ${formatTimeAgo(binary.modified)}`
												: "Modification time unavailable"}
										</p>
									</div>
								))}
							</div>
						) : (
							<p className="text-sm text-ink-faint">No durable binaries found in tools/bin.</p>
						)}
					</div>
				</section>

				<section className="rounded-xl border border-app-line bg-app-box">
					<div className="flex flex-col gap-3 border-b border-app-line px-4 py-3 md:flex-row md:items-center md:justify-between">
						<div>
							<h2 className="font-plex text-sm font-medium text-ink">Capabilities</h2>
							<p className="mt-1 text-xs text-ink-faint">
								Runtime tool truth, including process exposure, confidence, and remediation.
							</p>
						</div>
						<div className="flex flex-col gap-2 sm:flex-row sm:items-center">
							<div className="flex flex-wrap gap-1.5">
								{STATUS_FILTERS.map((filter) => (
									<button
										key={filter}
										onClick={() => setStatusFilter(filter)}
										className={`rounded-full px-2.5 py-1 text-tiny font-medium transition-colors ${
											statusFilter === filter
												? "bg-app-hover text-ink"
												: "text-ink-faint hover:bg-app-hover hover:text-ink-dull"
										}`}
									>
										{filter}
									</button>
								))}
							</div>
							<input
								type="text"
								value={search}
								onChange={(event) => setSearch(event.target.value)}
								placeholder="Search capabilities..."
								className="h-8 rounded-md border border-app-line/60 bg-app-input px-2.5 text-xs text-ink placeholder:text-ink-faint focus:border-accent/50 focus:outline-none"
							/>
						</div>
					</div>
					<div className="divide-y divide-app-line/40">
						{toolsQuery.isLoading ? (
							<div className="p-4 text-sm text-ink-faint">Loading capability snapshot...</div>
						) : toolsQuery.isError ? (
							<div className="p-4 text-sm text-red-400">Failed to load capability snapshot.</div>
						) : filteredCapabilities.length ? (
							filteredCapabilities.map((capability) => (
								<div key={capability.id} className="grid gap-3 px-4 py-4 md:grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)]">
									<div className="min-w-0">
										<div className="flex flex-wrap items-center gap-2">
											<h3 className="truncate text-sm font-medium text-ink">{capability.name}</h3>
											<Badge variant={statusVariant(capability.status)}>{capability.status}</Badge>
											<Badge variant="outline">{capability.subject.replace(/_/g, " ")}</Badge>
										</div>
										<p className="mt-2 break-all font-mono text-xs text-ink-faint">
											{capability.invocation_path}
										</p>
										{capability.detail && (
											<p className="mt-2 text-sm text-ink-dull">{capability.detail}</p>
										)}
									</div>
									<div className="grid gap-2 text-xs text-ink-faint">
										<div className="flex flex-wrap items-center gap-2">
											<Badge variant="blue">{confidenceLabel(capability.confidence)}</Badge>
											<span>Processes: {capability.process_types.join(", ")}</span>
										</div>
										{capability.reason_code && <p>Reason: {capability.reason_code}</p>}
										{capability.remediation && <p>Fix: {capability.remediation}</p>}
										{capability.source && <p>Source: {capability.source}</p>}
										{capability.last_verified_at && (
											<p>Last verified {formatTimeAgo(capability.last_verified_at)}</p>
										)}
									</div>
								</div>
							))
						) : (
							<div className="p-4 text-sm text-ink-faint">No capabilities match this filter.</div>
						)}
					</div>
				</section>
			</div>
		</div>
	);
}

function SummaryCard({
	label,
	value,
	tone,
	meta,
}: {
	label: string;
	value: number;
	tone: "green" | "amber" | "red" | "default";
	meta?: string;
}) {
	const borderClass =
		tone === "green"
			? "border-emerald-500/30"
			: tone === "amber"
				? "border-amber-500/30"
				: tone === "red"
					? "border-red-500/30"
					: "border-app-line";
	const textClass =
		tone === "green"
			? "text-emerald-400"
			: tone === "amber"
				? "text-amber-400"
				: tone === "red"
					? "text-red-400"
					: "text-ink";

	return (
		<div className={`rounded-xl border bg-app-box p-4 ${borderClass}`}>
			<p className="text-tiny uppercase tracking-[0.16em] text-ink-faint">{label}</p>
			<p className={`mt-2 font-plex text-2xl font-medium ${textClass}`}>{value}</p>
			{meta && <p className="mt-2 text-xs text-ink-faint">{meta}</p>}
		</div>
	);
}
