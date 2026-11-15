<script lang="ts" module>
	export type Tone = 'ok' | 'busy' | 'warn' | 'error' | 'idle';
</script>

<script lang="ts">
	let { tone, label }: { tone: Tone; label?: string } = $props();

	const colour: Record<Tone, string> = {
		ok: 'bg-emerald-500',
		busy: 'bg-amber-500 animate-pulse',
		warn: 'bg-amber-500',
		error: 'bg-destructive',
		idle: 'bg-muted-foreground/40'
	};
</script>

<span class="inline-block size-2 shrink-0 rounded-full {colour[tone]}" role={label ? 'img' : undefined} aria-label={label} title={label}></span>
