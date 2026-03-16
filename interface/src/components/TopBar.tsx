import { createContext, useContext, useRef, useCallback, useLayoutEffect, useSyncExternalStore, type ReactNode, type MouseEvent } from "react";
import { Link } from "@tanstack/react-router";
import { Menu01Icon, Moon02Icon, Sun03Icon } from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { BASE_PATH, IS_TAURI } from "@/api/client";
import { useTheme } from "@/hooks/useTheme";
import { Toggle } from "@/ui";

// ── Context ──────────────────────────────────────────────────────────────

interface TopBarStore {
	content: ReactNode;
	subscribe: (cb: () => void) => () => void;
	setContent: (node: ReactNode) => void;
	getSnapshot: () => ReactNode;
}

function createTopBarStore(): TopBarStore {
	let content: ReactNode = null;
	const listeners = new Set<() => void>();

	return {
		get content() {
			return content;
		},
		subscribe(cb: () => void) {
			listeners.add(cb);
			return () => listeners.delete(cb);
		},
		setContent(node: ReactNode) {
			content = node;
			for (const cb of listeners) cb();
		},
		getSnapshot() {
			return content;
		},
	};
}

const TopBarContext = createContext<TopBarStore | null>(null);

export function TopBarProvider({ children }: { children: ReactNode }) {
	const storeRef = useRef<TopBarStore>(null);
	if (!storeRef.current) {
		storeRef.current = createTopBarStore();
	}
	return (
		<TopBarContext.Provider value={storeRef.current}>
			{children}
		</TopBarContext.Provider>
	);
}

/**
 * Hook for routes to set topbar content. Content is displayed in the TopBar
 * component rendered at the root layout level.
 *
 * The component that calls this hook "owns" the topbar content for its lifetime.
 * Uses a ref + effect to avoid re-render loops.
 */
export function useSetTopBar(node: ReactNode) {
	const store = useContext(TopBarContext);
	if (!store) throw new Error("useSetTopBar must be used within TopBarProvider");

	useLayoutEffect(() => {
		store.setContent(node);
		return () => {
			store.setContent(null);
		};
	}, [node, store]);
}

// ── Component ────────────────────────────────────────────────────────────

function ThemeModeToggle() {
	const { isLightMode, toggleLightMode } = useTheme();

	return (
		<div className="mr-2 flex items-center gap-2 rounded-full border border-app-line bg-app-box/80 px-2.5 py-1.5 md:mr-4 md:px-3">
			<HugeiconsIcon
				icon={isLightMode ? Sun03Icon : Moon02Icon}
				className="h-3.5 w-3.5 text-ink-faint"
			/>
			<span className="hidden text-xs font-medium text-ink-dull sm:inline">
				{isLightMode ? "Light" : "Dark"}
			</span>
			<Toggle
				size="sm"
				checked={isLightMode}
				onCheckedChange={toggleLightMode}
				aria-label="Toggle light mode"
			/>
		</div>
	);
}

interface TopBarProps {
	isMobile?: boolean;
	onOpenMobileNav?: () => void;
}

export function TopBar({
	isMobile = false,
	onOpenMobileNav,
}: TopBarProps) {
	const store = useContext(TopBarContext);
	if (!store) throw new Error("TopBar must be used within TopBarProvider");

	const content = useSyncExternalStore(
		store.subscribe,
		store.getSnapshot,
		store.getSnapshot,
	);

	const handleMouseDown = useCallback((e: MouseEvent) => {
		if (!IS_TAURI) return;
		// Only drag on primary button, and not when clicking interactive elements
		if (e.buttons !== 1) return;
		const target = e.target as HTMLElement;
		if (target.closest("a, button, input, select, textarea, [role=button]")) return;
		e.preventDefault();
		(window as any).__TAURI_INTERNALS__.invoke("plugin:window|start_dragging");
	}, []);

	return (
		<div
			className={`flex min-h-12 w-full shrink-0 border-b border-app-line bg-app-darkBox/50${IS_TAURI ? " select-none" : ""}`}
			onMouseDown={handleMouseDown}
		>
			{IS_TAURI ? (
				<div className="flex w-[72px] shrink-0 items-center border-r border-sidebar-line bg-sidebar px-2">
					{isMobile && (
						<button
							type="button"
							onClick={onOpenMobileNav}
							className="flex h-8 w-8 items-center justify-center rounded-md text-sidebar-inkDull hover:bg-sidebar-selected/50 hover:text-sidebar-ink"
							aria-label="Open navigation"
						>
							<HugeiconsIcon icon={Menu01Icon} className="h-4 w-4" />
						</button>
					)}
				</div>
			) : isMobile ? (
				<div className="flex shrink-0 items-center border-r border-sidebar-line bg-sidebar px-2">
					<button
						type="button"
						onClick={onOpenMobileNav}
						className="flex h-8 w-8 items-center justify-center rounded-md text-sidebar-inkDull hover:bg-sidebar-selected/50 hover:text-sidebar-ink"
						aria-label="Open navigation"
					>
						<HugeiconsIcon icon={Menu01Icon} className="h-4 w-4" />
					</button>
				</div>
			) : (
				<Link
					to="/"
					className="flex w-12 shrink-0 items-center justify-center border-r border-sidebar-line bg-sidebar md:w-14"
				>
					<img
						src={`${BASE_PATH}/ball.png`}
						alt=""
						className="h-5 w-5 transition-transform duration-150 ease-out hover:scale-110 active:scale-95 md:h-6 md:w-6"
						draggable={false}
					/>
				</Link>
			)}

			<div className="flex min-w-0 flex-1 items-center overflow-hidden py-1">
				<div className="min-w-0 flex-1">{content}</div>
				<ThemeModeToggle />
			</div>
		</div>
	);
}
