-- cobra.als
-- Domain model for cobra (Go CLI framework).
-- Language-specific benchmark: Go struct composition, interface, large struct.

sig Command {
  use:               one Str,
  aliases:           seq Str,
  short:             one Str,
  long:              one Str,
  example:           one Str,
  groupID:           lone Str,
  version:           one Str,
  deprecated:        lone Str,
  hidden:            one Bool,
  commands:          seq Command,
  parent:            lone Command,
  completionOptions: one CompletionOptions,
  silenceErrors:     one Bool,
  silenceUsage:      one Bool,
  traverseChildren:  one Bool
}

sig Group {
  iD:    one Str,
  title: one Str
}

sig CompletionOptions {
  disableDefaultCmd:   one Bool,
  disableNoDescFlag:   one Bool,
  disableDescriptions: one Bool,
  hiddenDefaultCmd:    one Bool
}

abstract sig ShellCompDirective {}
one sig ShellCompDirectiveError         extends ShellCompDirective {}
one sig ShellCompDirectiveNoSpace       extends ShellCompDirective {}
one sig ShellCompDirectiveNoFileComp    extends ShellCompDirective {}
one sig ShellCompDirectiveFilterFileExt extends ShellCompDirective {}
one sig ShellCompDirectiveFilterDirs    extends ShellCompDirective {}
one sig ShellCompDirectiveKeepOrder     extends ShellCompDirective {}
one sig ShellCompDirectiveDefault       extends ShellCompDirective {}
