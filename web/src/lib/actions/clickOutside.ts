import type { Action } from 'svelte/action';

export const clickOutside: Action<HTMLElement, (event: MouseEvent) => void> = (
	node,
	handler
) => {
	const handleClick = (event: MouseEvent) => {
		if (node && !node.contains(event.target as Node) && !event.defaultPrevented) {
			handler(event);
		}
	};

	document.addEventListener('click', handleClick, true);

	return {
		destroy() {
			document.removeEventListener('click', handleClick, true);
		}
	};
};
