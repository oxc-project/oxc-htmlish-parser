<template>
  <div :class="a" @click.stop="go">{{ msg }}</div>
</template>

<script setup lang="ts">
const a = "<div>";
</script>

<style scoped>
.a > .b {}
</style>

<docs>
# Title <b>
</docs>
<template lang="pug">
div {{ x }}
</template>
<Script>raw</Script>
<img src="x">
