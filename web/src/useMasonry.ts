// La maçonnerie de la page du jour, sans bibliothèque : dans une grille à lignes de 1 px (`.cards`),
// chaque carte couvre autant de lignes que sa hauteur plus l'écart, et `grid-auto-flow: dense` la pose
// sous la plus courte des colonnes. Une carte qui change de taille (une carte chargée, « Voir plus ») ou
// qui arrive (un filtre) se remesure.

import { useLayoutEffect, useRef } from 'react'

/** Le nombre de lignes de 1 px d'une carte : sa hauteur, plus l'écart qui la sépare de la suivante. */
export const masonrySpan = (height: number, gap: number) => Math.max(1, Math.ceil(height + gap))

export function useMasonry<T extends HTMLElement>() {
  const ref = useRef<T>(null)

  useLayoutEffect(() => {
    const grid = ref.current
    if (!grid || typeof ResizeObserver === 'undefined') return
    const place = (card: Element) => {
      if (!(card instanceof HTMLElement)) return
      const gap = parseFloat(getComputedStyle(grid).columnGap) || 0
      card.style.gridRowEnd = `span ${masonrySpan(card.getBoundingClientRect().height, gap)}`
    }
    const sizes = new ResizeObserver((entries) => {
      for (const entry of entries) place(entry.target)
    })
    // Observer deux fois la même carte ne fait rien : on repasse toutes les cartes à chaque arrivée.
    const watch = () => {
      for (const card of grid.children) sizes.observe(card)
    }
    const arrivals = new MutationObserver(watch)
    // Mesurées dans la grille ordinaire (même largeur, même hauteur), avant de passer en maçonnerie :
    // la première image est déjà juste.
    for (const card of grid.children) place(card)
    grid.dataset.masonry = ''
    watch()
    arrivals.observe(grid, { childList: true })
    return () => {
      sizes.disconnect()
      arrivals.disconnect()
      delete grid.dataset.masonry
    }
  }, [])

  return ref
}
