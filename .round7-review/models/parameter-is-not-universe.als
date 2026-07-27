some sig P { x: one Int }

pred Pos[p: one P] {
  all q: P | q.x >= 0
}

assert LaterPos {
  eventually all p: P | Pos[p]
}
