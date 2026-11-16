import type { Settings } from '../api/index.js';
import { Baseline } from './baseline.js';

const plain = <V>(v: V): V => JSON.parse(JSON.stringify(v)) as V;
const validPort = (p: number) => Number.isInteger(p) && p >= 1 && p <= 65535;

/** Editable copy of the global settings. */
export class SettingsDraft {
	value = $state<Settings>({
		defaultPhpVersion: null,
		tld: '',
		apachePort: 80,
		services: [],
		loopbackPortRange: { start: 8100, end: 8199 },
		closeToTray: true,
		firstRunCompleted: false,
		defaultAddressing: 'no-elevation'
	});

	#baseline = new Baseline();

	constructor(source: Settings) {
		this.reset(source);
	}

	reset(source: Settings): void {
		this.value = plain(source);
		this.#baseline.set(this.value);
	}

	/** Takes `source` as the new baseline but keeps the edits, so saving overwrites it. */
	rebase(source: Settings): void {
		this.#baseline.set(source);
	}

	/** True when `source` (from the backend) differs from what this draft was loaded from. */
	isStale(source: Settings): boolean {
		return !this.#baseline.matches(source);
	}

	get dirty(): boolean {
		return !this.#baseline.matches(this.value);
	}

	get rangeValid(): boolean {
		const { start, end } = this.value.loopbackPortRange;
		return Number.isInteger(start) && Number.isInteger(end) && start >= 1024 && end <= 65535 && start < end;
	}

	get valid(): boolean {
		const v = this.value;
		return this.rangeValid && v.tld.trim().length > 0 && validPort(v.apachePort) && v.services.every((i) => validPort(i.port) && (i.uiPort == null || validPort(i.uiPort)));
	}

	toSettings(): Settings {
		return { ...plain(this.value), tld: this.value.tld.trim().replace(/^\./, '') };
	}
}
