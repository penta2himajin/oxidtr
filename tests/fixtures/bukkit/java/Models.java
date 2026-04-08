// Based on Bukkit (Minecraft server API).
// Hand-written Java exercising: abstract class, interface, deep hierarchy,
// enums with fields, generics.

package org.bukkit;

import java.util.List;

public abstract class Entity {
    private Location location;
    private World world;

    public Location getLocation() { return location; }
    public World getWorld() { return world; }
}

public class Player extends Entity {
    private String name;
    private int health;
    private int foodLevel;
    private GameMode gameMode;
    private PlayerInventory inventory;

    public String getName() { return name; }
    public int getHealth() { return health; }
    public int getFoodLevel() { return foodLevel; }
    public GameMode getGameMode() { return gameMode; }
    public PlayerInventory getInventory() { return inventory; }
}

public class World {
    private String name;
    private long seed;
    private Environment environment;
    private Difficulty difficulty;
    private int maxHeight;

    public String getName() { return name; }
    public long getSeed() { return seed; }
    public Environment getEnvironment() { return environment; }
    public Difficulty getDifficulty() { return difficulty; }
    public int getMaxHeight() { return maxHeight; }
}

public class Location {
    private World world;
    private double x;
    private double y;
    private double z;

    public Location(World world, double x, double y, double z) {
        this.world = world;
        this.x = x;
        this.y = y;
        this.z = z;
    }

    public World getWorld() { return world; }
    public double getX() { return x; }
    public double getY() { return y; }
    public double getZ() { return z; }
}

public class Block {
    private Material blockType;
    private Location location;

    public Material getType() { return blockType; }
    public Location getLocation() { return location; }
}

public class ItemStack {
    private Material material;
    private int amount;

    public ItemStack(Material material, int amount) {
        this.material = material;
        this.amount = amount;
    }

    public Material getType() { return material; }
    public int getAmount() { return amount; }
}

public class PlayerInventory {
    private int size;
    private List<ItemStack> contents;

    public int getSize() { return size; }
    public List<ItemStack> getContents() { return contents; }
}

public enum GameMode {
    SURVIVAL,
    CREATIVE,
    ADVENTURE,
    SPECTATOR
}

public enum Environment {
    NORMAL,
    NETHER,
    THE_END
}

public enum Difficulty {
    PEACEFUL,
    EASY,
    MEDIUM,
    HARD
}

public abstract class Material {}

public abstract class Event {
    private String eventName;
    public String getEventName() { return eventName; }
}

public class PlayerJoinEvent extends Event {
    private Player player;
    public Player getPlayer() { return player; }
}

public class BlockBreakEvent extends Event {
    private Player player;
    private Block block;

    public Player getPlayer() { return player; }
    public Block getBlock() { return block; }
}
