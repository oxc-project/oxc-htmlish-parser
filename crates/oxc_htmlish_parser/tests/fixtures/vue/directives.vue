<template>
  <C v-if="a" v-else-if="b" v-else v-for="x in xs" :key="x" />
  <C v-bind:id="a" :[name]="b" :[a.b].camel="c" .prop="d" v-bind="obj" />
  <C @click="f" v-on:keyup.enter.prevent="g" @[evt]="h" />
  <C #default="{ item }" #item.name="s" v-slot:header v-model.trim="m" />
  <C : @ v- v-bind: :[open plain />
  <div :a="1" v-pre>{{ raw }}<span :b="2">{{ raw }}</span></div>
  <p>{{ after }}</p>
</template>
