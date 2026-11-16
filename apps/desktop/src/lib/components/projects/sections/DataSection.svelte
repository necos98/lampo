<script lang="ts">
	import { onMount } from 'svelte';

	import type { Project, ServiceKindInfo } from '#lib/api/index.js';
	import CopyField from '#lib/components/common/CopyField.svelte';
	import FormRow from '#lib/components/common/FormRow.svelte';
	import SelectField from '#lib/components/common/SelectField.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import { messages as m } from '#lib/messages/index.js';
	import { notifier } from '#lib/services/notifier.js';
	import { copyText, openExternal } from '#lib/services/platform.js';
	import { settings } from '#lib/stores/index.js';
	import type { ProjectDraft } from '#lib/stores/projectDraft.svelte.js';
	import { DATABASE_KIND, serviceVersion, setService, usesService } from '#lib/projects/services.js';

	let { draft, project }: { draft: ProjectDraft; project: Project } = $props();
	const t = m.projects.data;

	// The kinds come from the backend (`list_service_kinds`): a new kind shows up here by itself.
	onMount(() => {
		if (settings.serviceKinds.data === null) void settings.serviceKinds.load();
	});
	const kinds = $derived(settings.serviceKinds.data ?? []);

	function versionOptions(kind: ServiceKindInfo) {
		const current = serviceVersion(draft.environment, kind.kind);
		return [
			{ value: '', label: t.serviceNone },
			...kind.versions.map((v) => ({ value: v, label: t.serviceVersion(kind.label, v) })),
			// A project may reference a series that is no longer configured: keep it visible.
			...(current && !kind.versions.includes(current) ? [{ value: current, label: t.serviceVersion(kind.label, current) }] : [])
		];
	}

	const db = $derived(project.database);
	const envBlock = $derived(
		db ? ['DB_CONNECTION=mysql', `DB_HOST=${db.host}`, `DB_PORT=${db.port}`, `DB_DATABASE=${db.database}`, `DB_USERNAME=${db.user}`, `DB_PASSWORD=${db.password}`].join('\n') : ''
	);

	async function copyEnv() {
		if (await copyText(envBlock)) notifier.success(m.common.copied);
		else notifier.error(m.common.copyFailed);
	}

	const mail = $derived(project.mail ?? null);
	// Mailpit takes plain SMTP: no credentials, no encryption.
	const mailEnvBlock = $derived(
		mail ? ['MAIL_MAILER=smtp', `MAIL_HOST=${mail.host}`, `MAIL_PORT=${mail.smtpPort}`, 'MAIL_USERNAME=null', 'MAIL_PASSWORD=null', 'MAIL_ENCRYPTION=null'].join('\n') : ''
	);

	async function copyMailEnv() {
		if (await copyText(mailEnvBlock)) notifier.success(m.common.copied);
		else notifier.error(m.common.copyFailed);
	}
</script>

{#if settings.serviceKinds.data === null}
	<p class="text-sm text-muted-foreground">{t.servicesLoading}</p>
{/if}
{#each kinds as kind (kind.kind)}
	{#if kind.versioned}
		<FormRow label={kind.label} for="service-{kind.kind}">
			<SelectField
				id="service-{kind.kind}"
				value={serviceVersion(draft.environment, kind.kind) ?? ''}
				options={versionOptions(kind)}
				onchange={(v) => setService(draft.environment, kind.kind, v !== '', v === '' ? null : v)}
			/>
		</FormRow>
		{#if kind.kind === DATABASE_KIND && serviceVersion(draft.environment, kind.kind)}
			<FormRow label={t.databaseName} for="dbname">
				<Input id="dbname" class="h-8 max-w-xs font-mono text-xs" value={draft.databaseName ?? ''} oninput={(e) => (draft.databaseName = e.currentTarget.value)} />
			</FormRow>
		{/if}
	{:else}
		<FormRow label={kind.label} help={t.serviceHelp(kind.label)}>
			{#snippet children(field)}
				<Switch {...field} checked={usesService(draft.environment, kind.kind)} onCheckedChange={(on) => setService(draft.environment, kind.kind, on)} />
			{/snippet}
		</FormRow>
	{/if}
{/each}

<div class="mt-4 rounded-md border bg-card p-3">
	<div class="mb-2 flex items-center justify-between">
		<h3 class="text-sm font-medium">{m.projects.database}</h3>
		{#if db}<Button variant="outline" size="xs" onclick={copyEnv}>{m.projects.copyEnv}</Button>{/if}
	</div>
	{#if db}
		<div class="flex flex-col gap-1">
			<CopyField label={m.common.host} value={db.host} />
			<CopyField label={m.common.port} value={String(db.port)} />
			<CopyField label={m.common.user} value={db.user} />
			<CopyField label={m.common.password} value={db.password} secret />
			<CopyField label={t.database} value={db.database} />
		</div>
		<p class="mt-2 text-xs text-muted-foreground">{m.projects.envNotice}</p>
	{:else}
		<p class="text-sm text-muted-foreground">{t.noDatabase}</p>
	{/if}
</div>

<div class="mt-4 rounded-md border bg-card p-3">
	<div class="mb-2 flex items-center justify-between">
		<h3 class="text-sm font-medium">{t.mail}</h3>
		{#if mail}
			<div class="flex gap-1.5">
				<Button variant="outline" size="xs" onclick={() => openExternal(mail.webUrl)}>{t.openWebUi}</Button>
				<Button variant="outline" size="xs" onclick={copyMailEnv}>{t.copyMailEnv}</Button>
			</div>
		{/if}
	</div>
	{#if mail}
		<div class="flex flex-col gap-1">
			<CopyField label={t.smtp} value="{mail.host}:{mail.smtpPort}" />
			<CopyField label={t.webUi} value={mail.webUrl} />
		</div>
		<p class="mt-2 text-xs text-muted-foreground">{t.mailHelp}</p>
	{:else}
		<p class="text-sm text-muted-foreground">{t.noMail}</p>
	{/if}
</div>
