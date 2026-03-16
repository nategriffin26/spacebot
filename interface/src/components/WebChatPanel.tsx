import {useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState} from "react";
import {Link} from "@tanstack/react-router";
import {useQuery} from "@tanstack/react-query";
import {api} from "@/api/client";
import {useWebChat} from "@/hooks/useWebChat";
import {isOpenCodeWorker, type ActiveWorker} from "@/hooks/useChannelLiveState";
import {useLiveContext} from "@/hooks/useLiveContext";
import {Markdown} from "@/components/Markdown";
import {seedGradient} from "@/components/ProfileAvatar";
import {formatTimestamp} from "@/lib/format";

interface WebChatPanelProps {
	agentId: string;
}

const chatScrollState = new Map<string, {scrollTop: number; stickToBottom: boolean}>();
const BOTTOM_THRESHOLD = 24;

function markersEqual(left: ChatOverviewMarker[], right: ChatOverviewMarker[]) {
	if (left.length !== right.length) return false;
	return left.every((marker, index) => {
		const other = right[index];
		return marker.id === other.id
			&& marker.color === other.color
			&& Math.abs(marker.top - other.top) < 0.1
			&& Math.abs(marker.targetTop - other.targetTop) < 1;
	});
}

interface ChatOverviewMarker {
	id: string;
	color: string;
	top: number;
	targetTop: number;
}

function isNearBottom(element: HTMLDivElement) {
	return element.scrollHeight - element.scrollTop - element.clientHeight <= BOTTOM_THRESHOLD;
}

function ChatOverviewBar({
	markers,
	viewportTop,
	viewportHeight,
	onSeek,
}: {
	markers: ChatOverviewMarker[];
	viewportTop: number;
	viewportHeight: number;
	onSeek: (ratio: number) => void;
}) {
	const trackRef = useRef<HTMLButtonElement>(null);
	const draggingRef = useRef(false);

	const seekToPointer = useCallback((clientY: number) => {
		const track = trackRef.current;
		if (!track) return;
		const rect = track.getBoundingClientRect();
		if (rect.height <= 0) return;
		const ratio = Math.min(1, Math.max(0, (clientY - rect.top) / rect.height));
		onSeek(ratio);
	}, [onSeek]);

	if (markers.length === 0) return null;

	return (
		<div className="sticky top-6 hidden w-6 shrink-0 md:flex md:justify-center">
			<button
				ref={trackRef}
				type="button"
				aria-label="Chat overview"
				onPointerDown={(event) => {
					draggingRef.current = true;
					event.currentTarget.setPointerCapture(event.pointerId);
					seekToPointer(event.clientY);
				}}
				onPointerMove={(event) => {
					if (!draggingRef.current) return;
					seekToPointer(event.clientY);
				}}
				onPointerUp={(event) => {
					draggingRef.current = false;
					event.currentTarget.releasePointerCapture(event.pointerId);
				}}
				onPointerCancel={() => {
					draggingRef.current = false;
				}}
				className="relative h-[calc(100dvh-12rem)] min-h-40 w-3 rounded-full bg-app-darkBox/80 outline-none transition-colors hover:bg-app-darkBox focus-visible:ring-2 focus-visible:ring-accent"
			>
				{markers.map((marker) => (
					<div
						key={marker.id}
						className="absolute left-1/2 h-1.5 w-1.5 -translate-x-1/2 rounded-full opacity-90"
						style={{
							top: `${marker.top}%`,
							backgroundColor: marker.color,
						}}
					/>
				))}
				<div
					className="absolute inset-x-0.5 rounded-full border border-app-line bg-app-box/70 shadow-sm"
					style={{
						top: `${viewportTop}%`,
						height: `${viewportHeight}%`,
					}}
				/>
			</button>
		</div>
	);
}

function ActiveWorkersPanel({
	workers,
	agentId,
}: {
	workers: ActiveWorker[];
	agentId: string;
}) {
	const [isExpanded, setIsExpanded] = useState(false);

	if (workers.length === 0) return null;

	// Use neutral chrome when all workers are opencode, amber when all builtin, mixed stays amber
	const allOpenCode = workers.every(isOpenCodeWorker);
	const borderColor = allOpenCode
		? "border-app-line/70 bg-app-selected/40"
		: "border-amber-500/25 bg-amber-500/5";
	const headerColor = allOpenCode ? "text-ink" : "text-amber-200";
	const dotColor = allOpenCode ? "bg-ink-faint" : "bg-amber-400";
	const caretColor = allOpenCode ? "text-ink-faint" : "text-amber-300";

	return (
		<div className={`rounded-lg border px-3 py-2 ${borderColor}`}>
			<button
				type="button"
				onClick={() => setIsExpanded((current) => !current)}
				aria-expanded={isExpanded}
				className={`flex w-full items-center gap-2 text-left text-tiny ${headerColor}`}
			>
				<div className={`h-1.5 w-1.5 animate-pulse rounded-full ${dotColor}`} />
				<span className="flex-1">
					{workers.length} active worker{workers.length !== 1 ? "s" : ""}
				</span>
				<span className={`text-xs transition-transform ${caretColor} ${isExpanded ? "rotate-180" : ""}`}>
					▾
				</span>
			</button>
			{isExpanded && (
			<div className="mt-2 flex flex-col gap-1.5">
				{workers.map((worker) => {
					const oc = isOpenCodeWorker(worker);
					return (
						<Link
							key={worker.id}
							to="/agents/$agentId/workers"
							params={{agentId}}
							search={{worker: worker.id}}
							className={`flex min-w-0 flex-wrap items-center gap-2 rounded-md px-2.5 py-1.5 text-tiny transition-colors sm:flex-nowrap ${
								oc
									? "bg-app-selected/60 hover:bg-app-selected"
									: "bg-amber-500/10 hover:bg-amber-500/20"
							}`}
						>
							<div
								className={`h-1.5 w-1.5 animate-pulse rounded-full ${oc ? "bg-ink-faint" : "bg-amber-400"}`}
							/>
							<span
								className={`font-medium ${oc ? "text-ink" : "text-amber-300"}`}
							>
								Worker
							</span>
							<span className="min-w-0 flex-1 truncate text-ink-dull">
								{worker.task}
							</span>
							<span className="shrink-0 text-ink-faint">{worker.status}</span>
							{worker.currentTool && (
								<span
									className={`max-w-40 shrink-0 truncate ${oc ? "text-ink-faint" : "text-amber-400/80"}`}
								>
									{worker.currentTool}
								</span>
							)}
						</Link>
					);
				})}
			</div>
			)}
		</div>
	);
}

function ThinkingIndicator() {
	return (
		<div className="flex items-center gap-1.5 py-1">
			<span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-ink-faint" />
			<span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-ink-faint [animation-delay:0.2s]" />
			<span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-ink-faint [animation-delay:0.4s]" />
		</div>
	);
}

function FloatingChatInput({
	value,
	onChange,
	onSubmit,
	disabled,
	agentId,
}: {
	value: string;
	onChange: (value: string) => void;
	onSubmit: () => void;
	disabled: boolean;
	agentId: string;
}) {
	const textareaRef = useRef<HTMLTextAreaElement>(null);

	useEffect(() => {
		const textarea = textareaRef.current;
		if (!textarea) return;

		const adjustHeight = () => {
			textarea.style.height = "auto";
			const scrollHeight = textarea.scrollHeight;
			const maxHeight = 200;
			textarea.style.height = `${Math.min(scrollHeight, maxHeight)}px`;
			textarea.style.overflowY = scrollHeight > maxHeight ? "auto" : "hidden";
		};

		adjustHeight();
		textarea.addEventListener("input", adjustHeight);
		return () => textarea.removeEventListener("input", adjustHeight);
	}, [value]);

	const handleKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
		if (event.key === "Enter" && !event.shiftKey) {
			event.preventDefault();
			onSubmit();
		}
	};

	return (
		<div className="pointer-events-none absolute inset-x-0 bottom-0 flex justify-center bg-gradient-to-t from-app via-app/85 to-transparent px-3 pb-[calc(env(safe-area-inset-bottom)+0.75rem)] pt-8 sm:px-4 sm:pb-4">
			<div className="pointer-events-auto w-full max-w-3xl">
				<div className="rounded-[1.35rem] border border-app-line/50 bg-app-box/60 shadow-xl backdrop-blur-xl transition-colors duration-200 hover:border-app-line/70 sm:rounded-2xl">
					<div className="flex items-end gap-2 p-2.5 sm:p-3">
						<textarea
							ref={textareaRef}
							value={value}
							onChange={(event) => onChange(event.target.value)}
							onKeyDown={handleKeyDown}
							placeholder={
								disabled ? "Waiting for response..." : `Message ${agentId}...`
							}
							disabled={disabled}
							rows={1}
							className="flex-1 resize-none bg-transparent px-1 py-1.5 text-base md:text-sm text-ink placeholder:text-ink-faint/60 focus:outline-none disabled:opacity-40"
							style={{maxHeight: "200px"}}
						/>
						<button
							type="button"
							onClick={onSubmit}
							disabled={disabled || !value.trim()}
							className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-accent text-white transition-all duration-150 hover:bg-accent-deep disabled:opacity-30 disabled:hover:bg-accent"
						>
							<svg
								width="16"
								height="16"
								viewBox="0 0 24 24"
								fill="none"
								stroke="currentColor"
								strokeWidth="2"
								strokeLinecap="round"
								strokeLinejoin="round"
							>
								<path d="M12 19V5M5 12l7-7 7 7" />
							</svg>
						</button>
					</div>
				</div>
			</div>
		</div>
	);
}

export function WebChatPanel({agentId}: WebChatPanelProps) {
	const {sessionId, isSending, error, sendMessage} = useWebChat(agentId);
	const {liveStates} = useLiveContext();
	const {data: agentsData} = useQuery({
		queryKey: ["agents"],
		queryFn: api.agents,
		staleTime: 10_000,
	});
	const [input, setInput] = useState("");
	const scrollContainerRef = useRef<HTMLDivElement>(null);
	const messageColumnRef = useRef<HTMLDivElement>(null);
	const messageRefs = useRef<Record<string, HTMLDivElement | null>>({});
	const hasRestoredScrollRef = useRef(false);
	const stickToBottomRef = useRef(true);
	const [overviewMarkers, setOverviewMarkers] = useState<ChatOverviewMarker[]>([]);
	const [viewportTop, setViewportTop] = useState(0);
	const [viewportHeight, setViewportHeight] = useState(100);

	const liveState = liveStates[sessionId];
	const timeline = useMemo(() => liveState?.timeline ?? [], [liveState?.timeline]);
	const isTyping = liveState?.isTyping ?? false;
	const messageItems = useMemo(
		() => timeline.filter((item) => item.type === "message"),
		[timeline],
	);
	const overviewMessageItems = useMemo(
		() => messageItems.filter((item, index) => item.role === "user" && (index === 0 || messageItems[index - 1].role !== item.role)),
		[messageItems],
	);
	const activeWorkers = useMemo(
		() => Object.values(liveState?.workers ?? {}).filter((worker) => !worker.isIdle),
		[liveState?.workers],
	);
	const hasActiveWorkers = activeWorkers.length > 0;
	const agent = agentsData?.agents.find((item) => item.id === agentId);
	const [defaultAgentColor] = seedGradient(agentId);
	const userColor = agent?.gradient_start || defaultAgentColor;

	const syncOverview = useCallback(() => {
		const scrollElement = scrollContainerRef.current;
		const columnElement = messageColumnRef.current;
		if (!scrollElement || !columnElement) return;

		const totalHeight = Math.max(columnElement.scrollHeight, 1);
		const nextMarkers: ChatOverviewMarker[] = overviewMessageItems
			.map((item) => {
				const element = messageRefs.current[item.id];
				if (!element) return null;
				return {
					id: item.id,
					color: userColor,
					top: (element.offsetTop / totalHeight) * 100,
					targetTop: element.offsetTop,
				};
			})
			.filter((marker): marker is ChatOverviewMarker => marker !== null);

		const nextViewportHeight = Math.min(
			100,
			Math.max((scrollElement.clientHeight / totalHeight) * 100, 8),
		);
		const nextViewportTop = Math.min(
			100 - nextViewportHeight,
			(scrollElement.scrollTop / totalHeight) * 100,
		);

		setOverviewMarkers((current) => markersEqual(current, nextMarkers) ? current : nextMarkers);
		setViewportTop((current) => Math.abs(current - Math.max(0, nextViewportTop)) < 0.1 ? current : Math.max(0, nextViewportTop));
		setViewportHeight((current) => Math.abs(current - nextViewportHeight) < 0.1 ? current : nextViewportHeight);
	}, [overviewMessageItems, userColor]);

	const setMessageRef = useCallback((messageId: string) => {
		return (element: HTMLDivElement | null) => {
			if (element) {
				messageRefs.current[messageId] = element;
			} else {
				delete messageRefs.current[messageId];
			}
		};
	}, []);

	const seekOverview = useCallback((ratio: number) => {
		const scrollElement = scrollContainerRef.current;
		if (!scrollElement) return;
		const maxScrollTop = Math.max(0, scrollElement.scrollHeight - scrollElement.clientHeight);
		scrollElement.scrollTo({ top: ratio * maxScrollTop, behavior: "auto" });
	}, []);

	useEffect(() => {
		hasRestoredScrollRef.current = false;
		stickToBottomRef.current = true;
	}, [sessionId]);

	useLayoutEffect(() => {
		const element = scrollContainerRef.current;
		if (!element || hasRestoredScrollRef.current) return;

		const savedState = chatScrollState.get(sessionId);
		if (!savedState || savedState.stickToBottom) {
			element.scrollTop = element.scrollHeight;
			stickToBottomRef.current = true;
		} else {
			element.scrollTop = Math.min(
				savedState.scrollTop,
				Math.max(0, element.scrollHeight - element.clientHeight),
			);
			stickToBottomRef.current = isNearBottom(element);
		}

		hasRestoredScrollRef.current = true;
	}, [sessionId, timeline.length]);

	useLayoutEffect(() => {
		syncOverview();
	}, [syncOverview]);

	useEffect(() => {
		const scrollElement = scrollContainerRef.current;
		const columnElement = messageColumnRef.current;
		if (!scrollElement || !columnElement || typeof ResizeObserver === "undefined") return;

		const observer = new ResizeObserver(() => {
			syncOverview();
		});

		observer.observe(scrollElement);
		observer.observe(columnElement);
		for (const element of Object.values(messageRefs.current)) {
			if (element) {
				observer.observe(element);
			}
		}

		return () => observer.disconnect();
	}, [messageItems, syncOverview]);

	const saveScrollState = () => {
		const element = scrollContainerRef.current;
		if (!element) return;
		const stickToBottom = isNearBottom(element);
		stickToBottomRef.current = stickToBottom;
		chatScrollState.set(sessionId, {
			scrollTop: element.scrollTop,
			stickToBottom,
		});
	};

	useEffect(() => {
		if (!hasRestoredScrollRef.current || !stickToBottomRef.current) return;
		const element = scrollContainerRef.current;
		if (!element) return;
		element.scrollTo({top: element.scrollHeight, behavior: "smooth"});
		chatScrollState.set(sessionId, {
			scrollTop: element.scrollHeight,
			stickToBottom: true,
		});
	}, [sessionId, timeline.length, isTyping, activeWorkers.length]);

	useEffect(() => () => {
		saveScrollState();
	}, [sessionId]);

	const handleSubmit = () => {
		const trimmed = input.trim();
		if (!trimmed || isSending) return;
		setInput("");
		sendMessage(trimmed);
	};

	return (
		<div className="relative flex h-full w-full flex-col">
			{/* Messages */}
			<div
				ref={scrollContainerRef}
				onScroll={() => {
					saveScrollState();
					syncOverview();
				}}
				className="flex-1 overflow-x-hidden overflow-y-auto overscroll-y-contain"
			>
				<div className="mx-auto flex w-full max-w-[56rem] items-start gap-4 px-3 py-4 pb-36 sm:px-4 sm:py-6 sm:pb-32">
					<ChatOverviewBar
						markers={overviewMarkers}
						viewportTop={viewportTop}
						viewportHeight={viewportHeight}
						onSeek={seekOverview}
					/>

					<div ref={messageColumnRef} className="min-w-0 flex-1">
						{hasActiveWorkers && (
							<div className="sticky top-0 z-10 -mx-1 bg-app/90 px-1 pb-2 pt-2 backdrop-blur-sm sm:mx-0 sm:px-0">
								<ActiveWorkersPanel workers={activeWorkers} agentId={agentId} />
							</div>
						)}

						{timeline.length === 0 && !isTyping && (
							<div className="flex flex-col items-center justify-center py-20 sm:py-24">
								<div className="rounded-2xl border border-app-line bg-app-darkBox/60 px-6 py-8 text-center shadow-sm">
									<p className="text-sm text-ink-faint">
									Start a conversation with {agentId}
									</p>
								</div>
							</div>
						)}

						{messageItems.map((item, index) => {
							const previousItem = index > 0 ? messageItems[index - 1] : null;
							const spacingClass = index === 0 ? "" : previousItem?.role === item.role ? "mt-3 sm:mt-4" : "mt-4 sm:mt-5";
							const timestamp = formatTimestamp(new Date(item.created_at).getTime());

							return (
								<div key={item.id} ref={setMessageRef(item.id)} className={spacingClass}>
								{item.role === "user" ? (
									<div className="flex justify-end">
										<div className="max-w-[92%] min-w-0 sm:max-w-[85%]">
											<div
												className="overflow-hidden rounded-2xl rounded-br-md px-3.5 py-2.5 shadow-sm sm:px-4"
												style={{ backgroundColor: userColor }}
											>
												<p className="whitespace-pre-wrap break-words text-sm text-white">
													{item.content}
												</p>
											</div>
											<div className="pt-1 text-right text-tiny opacity-80" style={{ color: userColor }}>
												{timestamp}
											</div>
										</div>
									</div>
								) : (
									<div className="flex justify-start">
										<div className="max-w-[92%] min-w-0 sm:max-w-[85%]">
											<div className="overflow-hidden rounded-2xl rounded-bl-md border border-app-line/60 bg-app-darkBox px-3.5 py-2.5 shadow-sm sm:px-4">
												<Markdown className="text-sm text-ink">{item.content}</Markdown>
											</div>
											<div className="pt-1 text-tiny text-ink-faint">
												{timestamp}
											</div>
										</div>
									</div>
								)}
								</div>
							);
						})}

						{/* Typing indicator */}
						{isTyping && <ThinkingIndicator />}

						{error && (
							<div className="rounded-lg border border-red-500/20 bg-red-500/5 px-4 py-3 text-sm text-red-400">
								{error}
							</div>
						)}
					</div>
				</div>
			</div>

			{/* Floating input */}
			<FloatingChatInput
				value={input}
				onChange={setInput}
				onSubmit={handleSubmit}
				disabled={isSending || isTyping}
				agentId={agentId}
			/>
		</div>
	);
}
