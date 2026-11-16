import type { DoctorCheck, DoctorReport } from '../api/index.js';
import type { NavId } from '../services/navigation.js';

/** A fix the app runs itself, by name: the panel binds each to its store call. */
export type FixCommand = 'install-shims' | 'prune-worktrees' | 'apply-hosts' | 'start-services';

/** What the Doctor offers next to a failed check, beside the textual suggestion. */
export type FixAction =
	| { kind: 'command'; command: FixCommand }
	/** A page of the app where the user fixes it (`href` is its sidebar route). */
	| { kind: 'navigate'; href: string; page: NavId }
	/** A download in the default browser. */
	| { kind: 'download'; href: string; label: string };

const COMMAND_FIXES: Partial<Record<string, FixCommand>> = {
	'shims-on-path': 'install-shims',
	'stale-worktrees': 'prune-worktrees',
	'hosts-block': 'apply-hosts',
	services: 'start-services'
};

const PAGE_FIXES: Partial<Record<string, NavId>> = {
	packages: 'packages',
	folders: 'projects',
	ports: 'services'
};

/**
 * The actions that fix `check`, by its id. None for a passing check or one only the user can fix
 * outside Lampo (another program shadowing the shims on PATH).
 */
export function fixActions(check: DoctorCheck, report: Pick<DoctorReport, 'missingVcRuntimes'>): FixAction[] {
	if (check.status === 'ok') return [];
	if (check.id === 'vc-runtime') {
		return report.missingVcRuntimes.map((vc) => ({ kind: 'download', href: vc.downloadUrl, label: vc.redistributable }));
	}
	const command = COMMAND_FIXES[check.id];
	if (command) return [{ kind: 'command', command }];
	const page = PAGE_FIXES[check.id];
	return page ? [{ kind: 'navigate', href: `/${page}`, page }] : [];
}
