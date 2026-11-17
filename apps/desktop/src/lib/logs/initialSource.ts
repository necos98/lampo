import { pickInitialSource } from './lastSource.ts';
import { serviceLogSource } from './serviceSource.ts';

/** A listed source, as far as choosing the first one goes. */
export interface ListedSource {
	id: string;
	exists: boolean;
}

/** What the logs page was opened for: a source (a project's link) or a service (a toast's «Vedi log»). */
export interface SourceRequest {
	source: string | null;
	service: string | null;
}

export interface OpenRequestedSource {
	/** Loads the source list and hands it back. */
	load: () => Promise<readonly ListedSource[]>;
	request: SourceRequest;
	remembered: () => string | null;
	/** Opens a source and remembers it. */
	show: (source: string) => void;
	/** Aborted when the request is replaced or the viewer goes away. */
	signal: AbortSignal;
}

/**
 * Loads the sources, then shows the one `request` asks for (else the remembered one, else the
 * first with a file). Nothing is shown once `signal` aborted meanwhile: a page left before the
 * list arrived must not open a log nor change the remembered source.
 */
export async function openRequestedSource({ load, request, remembered, show, signal }: OpenRequestedSource): Promise<void> {
	const sources = await load();
	if (signal.aborted) return;
	const pick = pickInitialSource(sources, request.source ?? serviceLogSource(sources, request.service), remembered());
	if (pick) show(pick);
}
