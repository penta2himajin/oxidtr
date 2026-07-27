some sig C { var v: one Int }
some sig D { var v: one Int }

fact NestedStep {
  always all c: C | all d: D | d.v' = d.v
}
