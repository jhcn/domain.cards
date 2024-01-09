/** @type {import('tailwindcss').Config} */
module.exports = {
  content: ['./templates/*.{html,js}'],
  theme: {
    extend: {
      boxShadow: {
        bento: '0 2px 4px 0 rgb(0 0 0/.04)',
        avatar: '0 1px 2px 0 rgb(0 0 0/.04)'
      },
      borderRadius: {
        bento: '1.25rem'
      }
    },
  },
  plugins: [],
}

