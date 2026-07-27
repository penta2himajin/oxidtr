one sig C { var v: one Int }

fact SingletonStep {
  always some c: C | c.v' = c.v
}
