<template>
  <!-- A reference to a quote, decoded as HTML decodes a text's: without its `;` too -->
  <p>{{ '&quot;' }} {{ '&quot' }} {{ '&#39' }} {{ '&#x22;' }} {{ "&apos;" }}</p>
  <!-- None: another character, none at all -->
  <p>{{ '&times;' }} {{ a }}</p>
</template>
