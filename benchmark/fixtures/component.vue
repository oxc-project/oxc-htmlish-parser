<template>
  <section class="items" :class="{ active }" v-bind="attrs">
    <h1>{{ title }}</h1>
    <p v-if="items.length === 0">No items</p>
    <ul v-else>
      <li v-for="(item, index) in items" :key="item.id">
        <ItemCard :item="item" :[field].camel="item.value" @click.stop="select(item)">
          <template #default="{ value }"><span>{{ index }}: {{ value }}</span></template>
          <template #footer><button @click.prevent="remove(item)">Remove</button></template>
        </ItemCard>
      </li>
    </ul>
    <input v-model.trim="query" @keyup.enter="search">
    <div v-pre>{{ keep.as.written }} <custom-tag /></div>
  </section>
</template>
<script setup lang="ts">
const title = "Hello, 世界 🎉";
const query = "";
const items = [{ id: 1, value: "a < b" }];
</script>
<style scoped>
.items > ul { display: grid; gap: 1rem; }
</style>
