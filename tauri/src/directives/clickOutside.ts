export const vClickOutside = {
  mounted(el: HTMLElement & { _clickOutside?: (event: MouseEvent) => void }, binding: any) {
    el._clickOutside = (event: MouseEvent) => {
      if (!(el === event.target || el.contains(event.target as Node))) {
        binding.value()
      }
    }
    document.addEventListener('click', el._clickOutside)
  },
  unmounted(el: HTMLElement & { _clickOutside?: (event: MouseEvent) => void }) {
    if (el._clickOutside) {
      document.removeEventListener('click', el._clickOutside)
    }
  }
}
