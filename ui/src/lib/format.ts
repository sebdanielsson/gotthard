const UNITS = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];

export function formatBytes(bytes: number): string {
	let value = bytes;
	let unit = 0;
	while (value >= 1024 && unit < UNITS.length - 1) {
		value /= 1024;
		unit++;
	}
	return `${value.toFixed(unit === 0 ? 0 : 1)} ${UNITS[unit]}`;
}

export function formatUptime(seconds: number): string {
	const s = Math.floor(seconds);
	const days = Math.floor(s / 86400);
	const hours = Math.floor((s % 86400) / 3600);
	const minutes = Math.floor((s % 3600) / 60);
	const parts: string[] = [];
	if (days) parts.push(`${days}d`);
	if (days || hours) parts.push(`${hours}h`);
	parts.push(`${minutes}m`);
	return parts.join(' ');
}

export function formatTimestamp(unixSeconds: number): string {
	return new Date(unixSeconds * 1000).toLocaleString();
}
