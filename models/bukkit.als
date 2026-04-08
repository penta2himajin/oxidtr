-- bukkit.als
-- Domain model for Bukkit (Minecraft server API).
-- Language-specific benchmark: Java class hierarchy, interfaces, enums.

abstract sig Entity {
  location: one Location,
  world:    one World
}

sig Player extends Entity {
  name:      one Str,
  health:    one Int,
  foodLevel: one Int,
  gameMode:  one GameMode,
  inventory: one PlayerInventory
}

sig World {
  name:        one Str,
  seed:        one Int,
  environment: one Environment,
  difficulty:  one Difficulty,
  maxHeight:   one Int
}

sig Location {
  world: one World,
  x:     one Int,
  y:     one Int,
  z:     one Int
}

sig Block {
  blockType: one Material,
  location:  one Location
}

sig ItemStack {
  material: one Material,
  amount:   one Int
}

sig PlayerInventory {
  size:     one Int,
  contents: seq ItemStack
}

abstract sig GameMode {}
one sig SURVIVAL  extends GameMode {}
one sig CREATIVE  extends GameMode {}
one sig ADVENTURE extends GameMode {}
one sig SPECTATOR extends GameMode {}

abstract sig Environment {}
one sig NORMAL    extends Environment {}
one sig NETHER    extends Environment {}
one sig THE_END   extends Environment {}

abstract sig Difficulty {}
one sig PEACEFUL extends Difficulty {}
one sig EASY     extends Difficulty {}
one sig MEDIUM   extends Difficulty {}
one sig HARD     extends Difficulty {}

abstract sig Material {}

abstract sig Event {
  eventName: one Str
}

sig PlayerJoinEvent extends Event {
  player: one Player
}

sig BlockBreakEvent extends Event {
  player: one Player,
  block:  one Block
}
