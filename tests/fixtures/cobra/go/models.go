// Based on spf13/cobra's Command and related types.
// Hand-written Go exercising: large struct, pointer fields,
// slice fields, bool flags, self-referential tree.

package cobra

// Command is the primary type in cobra representing a CLI command.
type Command struct {
	Use               string
	Aliases           []string
	Short             string
	Long              string
	Example           string
	GroupID           *string
	Version           string
	Deprecated        *string
	Hidden            bool
	Commands          []*Command
	Parent            *Command
	CompletionOptions CompletionOptions
	SilenceErrors     bool
	SilenceUsage      bool
	TraverseChildren  bool
}

// Group represents a command group for help output.
type Group struct {
	ID    string
	Title string
}

// CompletionOptions controls shell completion behavior.
type CompletionOptions struct {
	DisableDefaultCmd   bool
	DisableNoDescFlag   bool
	DisableDescriptions bool
	HiddenDefaultCmd    bool
}

// ShellCompDirective controls completion directives.
type ShellCompDirective int

const (
	ShellCompDirectiveError ShellCompDirective = iota
	ShellCompDirectiveNoSpace
	ShellCompDirectiveNoFileComp
	ShellCompDirectiveFilterFileExt
	ShellCompDirectiveFilterDirs
	ShellCompDirectiveKeepOrder
	ShellCompDirectiveDefault
)
