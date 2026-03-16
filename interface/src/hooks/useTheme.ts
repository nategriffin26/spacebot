import { useCallback, useSyncExternalStore } from "react";

export type ThemeId = "default" | "vanilla" | "midnight" | "noir";

export interface ThemeOption {
	id: ThemeId;
	name: string;
	description: string;
	className: string;
}

export const THEMES: ThemeOption[] = [
	{
		id: "default",
		name: "Default",
		description: "Dark theme with purple accent",
		className: "",
	},
	{
		id: "vanilla",
		name: "Vanilla",
		description: "Light theme with blue accent",
		className: "vanilla-theme",
	},
	{
		id: "midnight",
		name: "Midnight",
		description: "Deep blue dark theme",
		className: "midnight-theme",
	},
	{
		id: "noir",
		name: "Noir",
		description: "Pure black and white theme",
		className: "noir-theme",
	},
];

const STORAGE_KEY = "spacebot-theme";
const LAST_DARK_STORAGE_KEY = "spacebot-last-dark-theme";
const DEFAULT_THEME_ID: ThemeId = "default";
const LIGHT_THEME_ID: ThemeId = "vanilla";

const listeners = new Set<() => void>();

function emitChange() {
	for (const listener of listeners) {
		listener();
	}
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

function isThemeId(value: string | null): value is ThemeId {
	return value !== null && THEMES.some((theme) => theme.id === value);
}

function isLightTheme(themeId: ThemeId) {
	return themeId === LIGHT_THEME_ID;
}

function getInitialTheme(): ThemeId {
	if (typeof window === "undefined") return DEFAULT_THEME_ID;
	const stored = localStorage.getItem(STORAGE_KEY);
	if (isThemeId(stored)) {
		return stored;
	}
	return DEFAULT_THEME_ID;
}

function getLastDarkTheme(): ThemeId {
	if (typeof window === "undefined") return DEFAULT_THEME_ID;
	const stored = localStorage.getItem(LAST_DARK_STORAGE_KEY);
	if (isThemeId(stored) && !isLightTheme(stored)) {
		return stored;
	}
	return DEFAULT_THEME_ID;
}

function rememberDarkTheme(themeId: ThemeId) {
	if (typeof window === "undefined" || isLightTheme(themeId)) return;
	localStorage.setItem(LAST_DARK_STORAGE_KEY, themeId);
}

function applyThemeClass(themeId: ThemeId) {
	const theme = THEMES.find((t) => t.id === themeId);
	const root = document.documentElement;

	THEMES.forEach((t) => {
		if (t.className) {
			root.classList.remove(t.className);
		}
	});

	if (theme?.className) {
		root.classList.add(theme.className);
	}

	root.dataset.themeMode = isLightTheme(themeId) ? "light" : "dark";
	root.style.colorScheme = isLightTheme(themeId) ? "light" : "dark";
}

function setThemeState(themeId: ThemeId) {
	currentTheme = themeId;

	if (typeof window !== "undefined") {
		localStorage.setItem(STORAGE_KEY, themeId);
		rememberDarkTheme(themeId);
		applyThemeClass(themeId);
	}

	emitChange();
}

function getSnapshot() {
	return currentTheme;
}

let currentTheme: ThemeId = getInitialTheme();

export function resolveThemeColor(variableName: string, fallback: string) {
	if (typeof window === "undefined") return fallback;
	const value = getComputedStyle(document.documentElement)
		.getPropertyValue(variableName)
		.trim();
	return value ? `hsl(${value})` : fallback;
}

export function resolveThemeColorWithAlpha(variableName: string, alpha: number, fallback: string) {
	if (typeof window === "undefined") return fallback;
	const value = getComputedStyle(document.documentElement)
		.getPropertyValue(variableName)
		.trim();
	return value ? `hsl(${value} / ${alpha})` : fallback;
}

export function useTheme() {
	const theme = useSyncExternalStore(subscribe, getSnapshot, () => DEFAULT_THEME_ID);

	const setTheme = useCallback((newTheme: ThemeId) => {
		setThemeState(newTheme);
	}, []);

	const toggleLightMode = useCallback(() => {
		if (isLightTheme(theme)) {
			setThemeState(getLastDarkTheme());
			return;
		}

		rememberDarkTheme(theme);
		setThemeState(LIGHT_THEME_ID);
	}, [theme]);

	return {
		theme,
		setTheme,
		themes: THEMES,
		isLightMode: isLightTheme(theme),
		toggleLightMode,
	};
}

if (typeof window !== "undefined") {
	currentTheme = getInitialTheme();
	applyThemeClass(currentTheme);

	window.addEventListener("storage", (event) => {
		if (event.key !== STORAGE_KEY) return;
		const nextTheme = getInitialTheme();
		if (nextTheme === currentTheme) return;
		currentTheme = nextTheme;
		applyThemeClass(nextTheme);
		emitChange();
	});
}
