# Motivation

THIS IS UNDER CONSTRUCTION

This design doc outlines the motivation and goals for building a new robotics framework.

There are several major constraints required for sufficiently complex robotics software:

1. Many distinct tasks must run concurrently.
2. The tasks need to communicate with each other.
3. The execution of specific tasks must adapt to the system's current state.
4. Some tasks must have exclusive control over hardware for the duration they are active.

The following section outlines various approaches to addressing these constraints used by existing frameworks.

## Actors

Definition from [wikipedia](https://en.wikipedia.org/wiki/Actor_model):

> The actor model in computer science is a mathematical model of concurrent computation that treats an actor as the basic building block of concurrent computation. In response to a message it receives, an actor can: make local decisions, create more actors, send more messages, and determine how to respond to the next message received. Actors may modify their own private state, but can only affect each other indirectly through messaging (removing the need for lock-based synchronization).

The actor model is suited to applications that need to perform computation in response to external state, which is exactly what robots do. It naturally fulfills all previously outlined requirements aside from #4, which can be fulfilled with a framework that ensures exclusive access to hardware (foreshadowing).

An example of an implementation of the actor model is in languages running on the [BEAM](https://en.wikipedia.org/wiki/BEAM_(Erlang_virtual_machine)) virtual machine (erlang, elixir), which execute processes in parallel that exchange messages.

### Graph Based

Similar approaches to BEAM languages are found in many robotics focused middleware frameworks, such as [ROS](https://ros.org/) (1, 2) and [DORA](https://dora-rs.ai/) (dora is unrelated to swiper*), which consist of long running processes (nodes) that send messages to each other over topics. These graph based frameworks are notably different from BEAM because they support multiple nodes interacting as separate programs, rather than multiple processes running as part of the same program. Communication between nodes is often facilitaed with an IPC system such as [DDS](https://en.wikipedia.org/wiki/Data_Distribution_Service), [Zenoh](https://zenoh.io/), or [iceoryx2](https://iceoryx.io/).

Graph based middlewares for robotics are a subset of the actor model because they often do not support nodes launching other nodes, or ending themselves.

The biggest strength of graph based execution systems for robotics is that they can completely decouple nodes from each other, allowing for straightforward interoperability between code written in different languages and by different organizations.

This model fulfills constraint 1 and 2 naturally, and fulfills 3 and 4 with external tools. Using ROS as an example, action clients/servers allow nodes to exclusively handle discrete actions, and ros2_control provides additional guarantees that only one controller will will control a hardware interface at a time.

There are a few drawbacks to a graph based execution model, however. Implementations that put a network or ipc boundary between nodes introduce additional serialization/deserialization overhead, in addition to often accepting a lot of complexity around node discovery and lifetime management. These have proven to be an acceptable cost for the benefit of interoperability in the case of the ROS ecosystem.

## Linear Execution

A much simpler model of execution is to simply have code execute in a perpetual main loop. This model lacks the capacity for multithreaded/parallel applications, but can execute concurrent tasks by running them sequentially inside the loop. [copper-rs](https://github.com/copper-project/copper-rs) takes advantage of this simple execution model to implement fully deterministic replay and optimize cache locality, while enforcing the decoupling of a graph based model, where each node produces interacts with input and output topics each tick.

Linear execution can more effectively represent dynamic task spawning, as in the actor model.

A side effect of their simplicity is that linear execution frameworks struggle to represent parallelism or tasks that execute at different rates, since each task must execute once per main loop tick. This can be overcome by splitting up computation into chunks and returning null values before the computation is finished, which naturally leads into the next section

show ros2_control as linear/async, rather than graph based because graph based is poorly suited to hardware requirements and realtime

## Asynchronous Execution

A modification of the linear execution model is to replace the sequential loop with an event loop that concurrently executes tasks and allows the tasks to not be executed every tick, but rather wait until a condition wakes them. This can much more effectively represent parallelism and tasks running at different rates, at the cost of less determinism when interacting with hardware(?). A notable drawback of both linear and asynchronous execution models is that they can't have decoupling over a network boundary like graph based middleware frameworks.

Since async tasks can be scheduled by other async tasks and chose when they stop, they provide a much cleaner abstraction for representing high level actions than is possible with state machines in graph models. However, dynamically spawning tasks requires

## Requirement Management

- ros2 control calls it resource management, resource is confusing because a lot of things can be called resource
- swiper calls it requirement stealing

for async tasks:

a) start immediately, don't wait on a lock
b) have sole ownership of the hardware they're using for the duration they're running

## WPILib Command Based

- requirement management
- lifecycles

## lifecycles

- behavior trees
- managed nodes
- action servers

kill lifecycles with

fn {
  start();
  loop {
    run();
  }
  end();
}

^ this is simpler, more intuitive, way more ergonomic

