import type { Project, ProjectWarning } from '../api/index.js';
import { messages as m } from '../messages/index.js';
import { DATABASE_KIND, serviceVersion } from './services.js';

/** Italian text of one of `project`'s warnings; the backend's English text for a code this GUI does not know. */
export function describeProjectWarning(project: Pick<Project, 'environment'>, warning: ProjectWarning): string {
	switch (warning.code) {
		case 'legacy-php-mysql-auth':
			return m.projects.warnings.legacyPhpMysqlAuth(project.environment.phpVersion, serviceVersion(project.environment, DATABASE_KIND) ?? '');
		default:
			return warning.message;
	}
}

/** The warnings of a saved project as one notice, or `null` when it has none. */
export function projectWarningsText(project: Pick<Project, 'environment' | 'warnings'>): string | null {
	return project.warnings.length ? project.warnings.map((w) => describeProjectWarning(project, w)).join(' ') : null;
}
